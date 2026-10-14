use crate::domain::application_layout::{Content, DropPlan, Placement, RegionId, Visibility};
use lgui::core::CursorIcon;
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub rect: UiRect,
    pub placement: Placement,
    /// A cell's body, as opposed to its edge, divider or window bands.
    pub center: bool,
    pub plan: Option<DropPlan>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MapScene {
    pub version: u64,
    pub viewport: UiRect,
    pub visibility: Visibility,
    pub sessions: Vec<u64>,
    pub cells: Vec<(RegionId, Content, UiRect)>,
    pub zones: Vec<Zone>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalDrag {
    pub session: u64,
    pub origin: Point,
    pub point: Point,
    pub active: bool,
    pub scene: Option<MapScene>,
    pub hover: Option<usize>,
    /// Validate the framework's hover state on the release frame. Its cancel
    /// event is also delivered as PointerUp, but clears all hovered nodes.
    pub release: Option<Point>,
}
/// What subscribers observe: a change counter and the cursor it implies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapSignal {
    pub revision: u64,
    pub cursor: CursorIcon,
}

use crate::domain::application_layout::Divider;
use lgui::core::{Observable, Point};
use lgui::prelude::UiRect;
use std::sync::Arc;
pub enum MapCommand {
    Begin {
        session: u64,
        point: Point,
    },
    Pointer {
        point: Point,
        viewport: UiRect,
    },
    Release {
        point: Point,
        viewport: UiRect,
    },
    Finish {
        point: Point,
        viewport: UiRect,
        hovered: bool,
    },
    Cancel,
    DividerStart(Divider),
    DividerDrag(Point),
    DividerEnd,
}
#[derive(Default)]
pub struct MapOutcome {
    pub consumed: bool,
    pub focus: bool,
}
type Inspect = dyn Fn(&mut dyn FnMut(Option<&TerminalDrag>)) + Send + Sync;
#[derive(Clone)]
pub struct MapBinding {
    inspect: Arc<Inspect>,
    signal: Arc<dyn Fn() -> Observable<MapSignal> + Send + Sync>,
    run: Arc<dyn Fn(MapCommand) -> MapOutcome + Send + Sync>,
    sash: Arc<dyn Fn(&Divider) -> bool + Send + Sync>,
}
impl MapBinding {
    pub fn new(
        inspect: impl Fn(&mut dyn FnMut(Option<&TerminalDrag>)) + Send + Sync + 'static,
        signal: impl Fn() -> Observable<MapSignal> + Send + Sync + 'static,
        run: impl Fn(MapCommand) -> MapOutcome + Send + Sync + 'static,
        sash: impl Fn(&Divider) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            inspect: Arc::new(inspect),
            signal: Arc::new(signal),
            run: Arc::new(run),
            sash: Arc::new(sash),
        }
    }
    pub fn with<R>(&self, read: impl FnOnce(Option<&TerminalDrag>) -> R) -> R {
        let mut read = Some(read);
        let mut result = None;
        (self.inspect)(&mut |drag| {
            result = Some(read.take().expect("read projection called twice")(drag))
        });
        result.expect("read projection was called")
    }
    pub fn snapshot(&self) -> Option<TerminalDrag> {
        self.with(|d| d.cloned())
    }
    pub fn observable(&self) -> Observable<MapSignal> {
        (self.signal)()
    }
    pub fn is_active(&self) -> bool {
        self.with(|d| d.is_some_and(|d| d.active))
    }
    pub fn is_dragging(&self) -> bool {
        self.with(|d| d.is_some())
    }
    pub fn cancel(&self) -> bool {
        self.run(MapCommand::Cancel).consumed
    }
    pub fn run(&self, command: MapCommand) -> MapOutcome {
        (self.run)(command)
    }
    pub fn sash_active(&self, divider: &Divider) -> bool {
        (self.sash)(divider)
    }
}
