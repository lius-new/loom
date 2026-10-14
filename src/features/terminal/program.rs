//! Program labels and candidate selection from supplied process information.
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramCandidate {
    pub pid: u32,
    pub name: String,
}

#[derive(Default)]
pub struct SustainedProgram {
    candidate: Option<ProgramCandidate>,
    since: Option<Instant>,
}

impl SustainedProgram {
    pub fn sustained_pid(&self, now: Instant) -> Option<u32> {
        self.since
            .filter(|since| now.duration_since(*since) >= Duration::from_secs(1))
            .and_then(|_| self.candidate.as_ref().map(|program| program.pid))
    }
    pub fn observe(&mut self, candidate: Option<ProgramCandidate>, now: Instant) -> Option<String> {
        if candidate != self.candidate {
            self.candidate = candidate;
            self.since = self.candidate.as_ref().map(|_| now);
        }
        self.since
            .filter(|since| now.duration_since(*since) >= Duration::from_secs(1))
            .and_then(|_| self.candidate.as_ref().map(|program| program.name.clone()))
    }
}

pub(crate) fn program_name(path: &str) -> Option<String> {
    let name = path.trim().rsplit(['/', '\\']).next()?;
    let name = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .unwrap_or(name);
    (!name.is_empty()).then(|| {
        name.chars()
            .filter(|ch| !ch.is_control())
            .take(64)
            .collect()
    })
}

/// Ownership survives exited parents. Collapse a single-child launch chain;
/// after a program is established, its later helpers cannot steal its label.
#[cfg(any(windows, test))]
pub(crate) fn session_program(
    members: &[u32],
    baseline: &[u32],
    preferred: Option<u32>,
    processes: &[(u32, u32, String)],
) -> Option<ProgramCandidate> {
    let programs = processes
        .iter()
        .filter(|(pid, _, _)| members.contains(pid) && !baseline.contains(pid))
        .collect::<Vec<_>>();
    let candidate = |entry: &(u32, u32, String)| {
        Some(ProgramCandidate {
            pid: entry.0,
            name: program_name(&entry.2)?,
        })
    };
    if let Some(entry) = preferred.and_then(|pid| programs.iter().find(|entry| entry.0 == pid)) {
        return candidate(entry);
    }
    let mut entry = *programs
        .iter()
        .find(|entry| !programs.iter().any(|parent| parent.0 == entry.1))?;
    let mut seen = Vec::new();
    loop {
        if seen.contains(&entry.0) {
            return None;
        }
        seen.push(entry.0);
        let children = programs
            .iter()
            .filter(|child| child.1 == entry.0)
            .collect::<Vec<_>>();
        if children.len() != 1 {
            return candidate(entry);
        }
        entry = children[0];
    }
}

/// Best-effort fallback for unavailable console membership. It knows only
/// process relationships, never application or shell names.
#[cfg(any(windows, test))]
pub(crate) fn descendant_program(
    root: u32,
    processes: &[(u32, u32, String)],
) -> Option<ProgramCandidate> {
    if root == 0 {
        return None;
    }
    let mut parents = vec![root];
    let mut visited = vec![root];
    while !parents.is_empty() {
        let mut next = Vec::new();
        for &(pid, parent, ref executable) in processes {
            if !parents.contains(&parent) || visited.contains(&pid) {
                continue;
            }
            visited.push(pid);
            let _ = executable;
            next.push(pid);
        }
        parents = next;
    }
    session_program(&visited, &[root], None, processes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_sustained_processes_get_labels_and_exit_resets_immediately() {
        let start = Instant::now();
        let app = ProgramCandidate {
            pid: 12,
            name: "node".into(),
        };
        let mut tracker = SustainedProgram::default();
        assert_eq!(tracker.observe(Some(app.clone()), start), None);
        assert_eq!(
            tracker.observe(Some(app.clone()), start + Duration::from_millis(999)),
            None
        );
        assert_eq!(
            tracker.observe(Some(app), start + Duration::from_secs(1)),
            Some("node".into())
        );
        assert_eq!(tracker.observe(None, start + Duration::from_secs(2)), None);
    }

    #[test]
    fn repeated_short_processes_and_program_switches_do_not_reuse_elapsed_time() {
        let start = Instant::now();
        let mut tracker = SustainedProgram::default();
        for pid in 1..10 {
            let app = ProgramCandidate {
                pid,
                name: "git".into(),
            };
            assert_eq!(
                tracker.observe(
                    Some(app),
                    start + Duration::from_millis(u64::from(pid) * 300)
                ),
                None
            );
        }
        let app = ProgramCandidate {
            pid: 42,
            name: "python".into(),
        };
        assert_eq!(
            tracker.observe(Some(app.clone()), start + Duration::from_secs(4)),
            None
        );
        assert_eq!(
            tracker.observe(Some(app), start + Duration::from_secs(5)),
            Some("python".into())
        );
    }

    #[test]
    fn ancestor_probe_follows_structural_launcher_chains() {
        let processes = vec![
            (2, 99, "unrelated-system-helper.exe".into()),
            (3, 1, "unknown-launcher.exe".into()),
            (4, 3, "C:\\tools\\node.exe".into()),
            (5, 99, "unrelated.exe".into()),
        ];
        assert_eq!(
            descendant_program(1, &processes),
            Some(ProgramCandidate {
                pid: 4,
                name: "node".into()
            })
        );
        assert_eq!(descendant_program(99, &[]), None);
        assert_eq!(program_name("/usr/bin/python"), Some("python".into()));
    }

    #[test]
    fn session_ownership_survives_exited_launchers_and_isolates_other_terminals() {
        let processes = vec![
            (1, 0, "bash.exe".into()),
            // Parent 10 has exited; names have no application-specific meaning.
            (11, 10, "unknown-launcher.exe".into()),
            (12, 11, "arbitrary-tool.exe".into()),
            (21, 20, "other-terminal.exe".into()),
        ];
        assert_eq!(descendant_program(1, &processes), None);
        assert_eq!(
            session_program(&[1, 11, 12], &[1], None, &processes),
            Some(ProgramCandidate {
                pid: 12,
                name: "arbitrary-tool".into()
            })
        );
        assert_eq!(session_program(&[1], &[1], None, &processes), None);
    }

    #[test]
    fn a_sustained_application_keeps_its_label_when_it_spawns_helpers() {
        let processes = vec![
            (1, 0, "bash.exe".into()),
            (12, 1, "arbitrary-tool.exe".into()),
            (13, 12, "unknown-helper.exe".into()),
        ];
        assert_eq!(
            session_program(&[1, 12, 13], &[1], Some(12), &processes),
            Some(ProgramCandidate {
                pid: 12,
                name: "arbitrary-tool".into()
            })
        );
        assert_eq!(session_program(&[1], &[1], Some(12), &processes), None);
    }
}
