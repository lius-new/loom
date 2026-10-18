//! Runs one language server process and its protocol session.
//!
//! Four threads cooperate: a reader decodes the server's output, a writer feeds
//! its input (so a server that stops reading can never stall the rest), a third
//! keeps the tail of its error output, and a driver owns the [`Session`], applies
//! its answers, and enforces the timeouts. The application only sends commands
//! and receives events; it never waits on the server.
use crate::features::language_service::{
    framing::{self, Decoder},
    message::{Id, Message, ParseError},
    session::{Event, Output, Session, SessionOptions, State},
};
use serde_json::Value;
use std::{
    collections::VecDeque,
    io::{BufRead, Read, Write},
    path::PathBuf,
    process::{Child, Command as Process, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Lines of the server's standard error kept for diagnosing a failure.
const STDERR_LINES: usize = 200;
const STDERR_LINE_BYTES: usize = 2000;

#[derive(Clone, Debug)]
pub(crate) struct Timeouts {
    /// From starting the process to the server's answer to initialize.
    pub initialize: Duration,
    /// For each request, unless the caller says otherwise.
    pub request: Duration,
    /// How long a server gets to exit after being told to, and to answer shutdown.
    pub exit_grace: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            initialize: Duration::from_secs(30),
            request: Duration::from_secs(5),
            exit_grace: Duration::from_secs(2),
        }
    }
}

pub(crate) struct Launch {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub session: SessionOptions,
    pub timeouts: Timeouts,
    pub max_message: usize,
}

enum Input {
    Message(Message),
    /// A message that framed correctly but is not valid JSON-RPC.
    BadMessage(String),
    /// The byte stream can no longer be followed.
    BadFrame(String),
    /// The server's output ended.
    Eof,
    /// Writing to the server failed, usually because it is gone.
    WriteFailed(String),
    Command(Command),
}

enum Command {
    Request {
        method: String,
        params: Value,
        tag: u64,
        timeout: Option<Duration>,
    },
    Notify {
        method: String,
        params: Value,
    },
    Shutdown,
}

/// A running server. Dropping it ends the process.
pub(crate) struct Client {
    input: Sender<Input>,
    driver: Option<JoinHandle<()>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    state: Arc<Mutex<State>>,
    process_id: u32,
}

impl Client {
    /// Starts the process and its threads. `on_event` runs on the driver thread
    /// and must not block for long.
    pub fn spawn(
        launch: Launch,
        on_event: impl Fn(Event) + Send + 'static,
    ) -> std::io::Result<Self> {
        let mut process = Process::new(&launch.command);
        process
            .args(&launch.args)
            .current_dir(&launch.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            // A background server must not open a console window.
            use std::os::windows::process::CommandExt;
            process.creation_flags(0x0800_0000);
        }
        let mut child = process.spawn()?;
        let process_id = child.id();
        let job = super::job::Job::adopt(&child);
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");

        let (input, inbox) = mpsc::channel::<Input>();
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let state = Arc::new(Mutex::new(State::Initializing));

        let reader_input = input.clone();
        let max_message = launch.max_message;
        thread::Builder::new()
            .name("loom-lsp-read".into())
            .spawn(move || read_loop(stdout, reader_input, max_message))?;
        let kept = tail.clone();
        thread::Builder::new()
            .name("loom-lsp-stderr".into())
            .spawn(move || keep_stderr(stderr, &kept))?;
        let (outbox, frames) = mpsc::channel::<Vec<u8>>();
        let writer_input = input.clone();
        thread::Builder::new()
            .name("loom-lsp-write".into())
            .spawn(move || write_loop(stdin, frames, writer_input))?;

        let driver_state = state.clone();
        let driver = thread::Builder::new()
            .name("loom-lsp".into())
            .spawn(move || {
                let (session, first) = Session::start(launch.session);
                Driver {
                    session,
                    inbox,
                    outbox,
                    child,
                    _job: job,
                    timeouts: launch.timeouts,
                    on_event: Box::new(on_event),
                    state: driver_state,
                    requests: VecDeque::new(),
                    initialize_deadline: None,
                    exit_deadline: None,
                    bad_messages: 0,
                    eof: false,
                }
                .run(first);
            })?;
        Ok(Self {
            input,
            driver: Some(driver),
            stderr: tail,
            state,
            process_id,
        })
    }

    /// Asks the server something; the answer arrives as an [`Event::Response`]
    /// carrying `tag`. A request the server does not answer in time fails.
    pub fn request(&self, method: &str, params: Value, tag: u64) -> bool {
        self.command(Command::Request {
            method: method.to_owned(),
            params,
            tag,
            timeout: None,
        })
    }

    pub fn notify(&self, method: &str, params: Value) -> bool {
        self.command(Command::Notify {
            method: method.to_owned(),
            params,
        })
    }

    /// Asks the server to stop. Returns at once; follow the state events, or call
    /// [`stop`](Self::stop) to wait.
    pub fn shutdown(&self) -> bool {
        self.command(Command::Shutdown)
    }

    fn command(&self, command: Command) -> bool {
        self.input.send(Input::Command(command)).is_ok()
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    /// The last lines the server wrote to standard error.
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr.lock().unwrap().iter().cloned().collect()
    }

    /// Stops the server and waits for the process to be gone.
    pub fn stop(mut self) {
        self.shutdown();
        self.join();
    }

    fn join(&mut self) {
        if let Some(driver) = self.driver.take() {
            let _ = driver.join();
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // The driver stops on its own once told to or once the channel closes; it
        // always ends the process before it returns.
        let _ = self.input.send(Input::Command(Command::Shutdown));
        self.join();
    }
}

fn read_loop(mut stdout: impl Read, input: Sender<Input>, max_message: usize) {
    let mut decoder = Decoder::new(max_message);
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = match stdout.read(&mut buffer) {
            Ok(0) | Err(_) => {
                let _ = input.send(Input::Eof);
                return;
            }
            Ok(read) => read,
        };
        match decoder.push(&buffer[..read]) {
            Ok(frames) => {
                for frame in frames {
                    let item = match Message::parse(&frame) {
                        Ok(message) => Input::Message(message),
                        Err(ParseError::Unaddressed) => continue,
                        Err(error) => Input::BadMessage(error.to_string()),
                    };
                    if input.send(item).is_err() {
                        return;
                    }
                }
            }
            Err(error) => {
                let _ = input.send(Input::BadFrame(error.to_string()));
                return;
            }
        }
    }
}

fn write_loop(mut stdin: impl Write, frames: Receiver<Vec<u8>>, input: Sender<Input>) {
    for frame in frames {
        if let Err(error) = stdin.write_all(&frame).and_then(|()| stdin.flush()) {
            let _ = input.send(Input::WriteFailed(error.to_string()));
            return;
        }
    }
    // The channel closed: dropping `stdin` tells the server we are done.
}

fn keep_stderr(stderr: impl Read, tail: &Mutex<VecDeque<String>>) {
    for line in std::io::BufReader::new(stderr).lines() {
        let Ok(mut line) = line else { return };
        line.truncate(line.floor_char_boundary(STDERR_LINE_BYTES));
        let mut tail = tail.lock().unwrap();
        if tail.len() == STDERR_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }
}

struct Driver {
    session: Session,
    inbox: Receiver<Input>,
    outbox: Sender<Vec<u8>>,
    child: Child,
    /// Ends the server's helper processes with it; dropped after the server.
    _job: Option<super::job::Job>,
    timeouts: Timeouts,
    on_event: Box<dyn Fn(Event) + Send>,
    state: Arc<Mutex<State>>,
    /// Pending request deadlines, oldest first (the timeout is the same for most).
    requests: VecDeque<(Instant, Id)>,
    initialize_deadline: Option<Instant>,
    /// Set once the server was told to stop: when to give up waiting for it.
    exit_deadline: Option<Instant>,
    bad_messages: usize,
    /// The server's output has ended.
    eof: bool,
}

impl Driver {
    fn run(mut self, first: Output) {
        self.initialize_deadline = Some(Instant::now() + self.timeouts.initialize);
        let mut terminate = self.apply(first);
        while !terminate {
            let input = match self.next_deadline() {
                Some(deadline) => match self
                    .inbox
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                {
                    Ok(input) => Some(input),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match self.inbox.recv() {
                    Ok(input) => Some(input),
                    Err(_) => break,
                },
            };
            let output = match input {
                Some(input) => self.input(input),
                None => self.timers(),
            };
            terminate = self.apply(output);
            // A failed session is over. A stopped one is over once the process has
            // gone, or at once if nobody is waiting for it to.
            let over = match self.session.state() {
                State::Failed(_) => true,
                State::Stopped => self.eof || self.exit_deadline.is_none(),
                _ => false,
            };
            if over {
                break;
            }
        }
        self.finish();
    }

    fn input(&mut self, input: Input) -> Output {
        match input {
            Input::Message(message) => self.session.receive(message),
            Input::BadMessage(_) => {
                // Framing held, so the stream is still usable; count it and go on.
                self.bad_messages += 1;
                Output::default()
            }
            Input::BadFrame(error) => self.session.fail(format!("protocol error: {error}")),
            Input::Eof => {
                self.eof = true;
                let reason = self.exit_reason();
                self.session.transport_closed(Some(reason))
            }
            Input::WriteFailed(error) => self
                .session
                .transport_closed(Some(format!("could not write to the server: {error}"))),
            Input::Command(command) => self.command(command),
        }
    }

    fn command(&mut self, command: Command) -> Output {
        match command {
            Command::Request {
                method,
                params,
                tag,
                timeout,
            } => match self.session.request(&method, params, tag) {
                Ok((id, output)) => {
                    let timeout = timeout.unwrap_or(self.timeouts.request);
                    self.requests.push_back((Instant::now() + timeout, id));
                    output
                }
                Err(error) => Output {
                    events: vec![Event::Response {
                        id: Id::Number(-1),
                        tag,
                        method,
                        outcome: Err(
                            crate::features::language_service::message::ResponseError::new(
                                crate::features::language_service::message::INTERNAL_ERROR,
                                format!("the server cannot take requests: {error:?}"),
                            ),
                        ),
                    }],
                    ..Output::default()
                },
            },
            Command::Notify { method, params } => {
                self.session.notify(&method, params).unwrap_or_default()
            }
            Command::Shutdown => {
                let output = self.session.begin_shutdown();
                if self.exit_deadline.is_none() && self.session.state() != &State::Initializing {
                    self.exit_deadline = Some(Instant::now() + self.timeouts.exit_grace);
                }
                output
            }
        }
    }

    fn timers(&mut self) -> Output {
        let now = Instant::now();
        let mut output = Output::default();
        if self.initialize_deadline.is_some_and(|d| d <= now) {
            self.initialize_deadline = None;
            if self.session.state() == &State::Initializing {
                let late = self
                    .session
                    .fail("timed out waiting for the server to initialize".into());
                merge(&mut output, late);
            }
        }
        while self
            .requests
            .front()
            .is_some_and(|(deadline, _)| *deadline <= now)
        {
            let (_, id) = self.requests.pop_front().unwrap();
            merge(&mut output, self.session.expire(&id));
        }
        if self.exit_deadline.is_some_and(|d| d <= now) {
            self.exit_deadline = None;
            // The server did not leave when asked, so it is ended. A stop that was
            // asked for still counts as a stop, not as a failure.
            merge(&mut output, self.session.transport_closed(None));
            output.terminate = true;
        }
        output
    }

    fn next_deadline(&self) -> Option<Instant> {
        [
            self.initialize_deadline,
            self.requests.front().map(|(deadline, _)| *deadline),
            self.exit_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Sends what the session produced and delivers its events. Returns whether
    /// the process must be ended now.
    fn apply(&mut self, output: Output) -> bool {
        for message in &output.send {
            if self
                .outbox
                .send(framing::encode(&message.to_bytes()))
                .is_err()
            {
                break;
            }
        }
        for event in output.events {
            if let Event::StateChanged(state) = &event {
                *self.state.lock().unwrap() = state.clone();
                if *state != State::Initializing {
                    self.initialize_deadline = None;
                }
                if state.is_closed() && !matches!(state, State::Stopped) {
                    self.requests.clear();
                }
            }
            if let Event::Response { id, .. } = &event {
                self.requests.retain(|(_, pending)| pending != id);
            }
            (self.on_event)(event);
        }
        output.terminate
    }

    /// Ends the process: politely if it is leaving, by force otherwise.
    fn finish(self) {
        // Closing the channel ends the writer, which closes the server's input.
        let Self {
            outbox, mut child, ..
        } = self;
        drop(outbox);
        let grace = Instant::now() + Duration::from_millis(300);
        while Instant::now() < grace {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Describes how the process ended, once it has had a moment to.
    fn exit_reason(&mut self) -> String {
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return match status.code() {
                        Some(code) => format!("the server exited with code {code}"),
                        None => "the server was terminated".to_owned(),
                    };
                }
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
        "the server closed the connection".to_owned()
    }
}

fn merge(into: &mut Output, more: Output) {
    into.send.extend(more.send);
    into.events.extend(more.events);
    into.terminate |= more.terminate;
}
