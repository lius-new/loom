//! Expansion of the placeholders a server definition starts from.
//!
//! A catalog command, its arguments and its initialization options name the
//! installed files they need: `${pack}` is the directory of the server's own pack,
//! `${runtime.<id>}` the directory of a pack it requires. Nothing else is expanded,
//! and a placeholder whose directory is not installed is an error, so a definition
//! can never start a program that was looked up anywhere else.
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Where the directories the placeholders stand for are.
pub struct Installed<'a> {
    /// The server's own pack.
    pub pack: &'a Path,
    /// The directory of an installed pack that was required, by id.
    pub runtime: &'a dyn Fn(&str) -> Option<PathBuf>,
}

/// `text` with every placeholder replaced by its directory.
pub fn expand(text: &str, installed: &Installed) -> Result<String, String> {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(format!("unterminated placeholder in `{text}`"));
        };
        let name = &after[..end];
        let directory = match name {
            "pack" => installed.pack.to_path_buf(),
            _ => {
                let id = name
                    .strip_prefix("runtime.")
                    .ok_or_else(|| format!("unknown placeholder `${{{name}}}`"))?;
                (installed.runtime)(id)
                    .ok_or_else(|| format!("the runtime `{id}` is not installed"))?
            }
        };
        result.push_str(&directory.to_string_lossy());
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

/// A JSON value with the placeholders in its strings expanded. Done on the parsed
/// value, because a Windows path put into the text of a string would not be valid
/// JSON.
pub fn expand_json(value: &Value, installed: &Installed) -> Result<Value, String> {
    Ok(match value {
        Value::String(text) => Value::String(expand(text, installed)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| expand_json(item, installed))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| Ok((key.clone(), expand_json(item, installed)?)))
                .collect::<Result<_, String>>()?,
        ),
        other => other.clone(),
    })
}

/// The program to start for an expanded command. On Windows a command names an
/// executable without its extension, and the file has `.exe`.
pub fn program(command: &str, windows: bool) -> PathBuf {
    let path = if windows {
        PathBuf::from(command.replace('/', "\\"))
    } else {
        PathBuf::from(command)
    };
    if windows && path.extension().is_none() {
        let with = path.with_extension("exe");
        if with.is_file() || !path.is_file() {
            return with;
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn runtimes(id: &str) -> Option<PathBuf> {
        (id == "node").then(|| PathBuf::from("/data/node"))
    }

    fn installed() -> Installed<'static> {
        Installed {
            pack: Path::new("/data/pack"),
            runtime: &runtimes,
        }
    }

    #[test]
    fn a_pack_and_a_required_runtime_expand_to_their_directories() {
        let installed = installed();
        assert_eq!(
            expand("${runtime.node}/node", &installed).unwrap(),
            "/data/node/node"
        );
        assert_eq!(
            expand("${pack}/a.js --root=${pack}", &installed).unwrap(),
            "/data/pack/a.js --root=/data/pack"
        );
        assert_eq!(expand("plain", &installed).unwrap(), "plain");
        assert_eq!(expand("", &installed).unwrap(), "");
    }

    #[test]
    fn nothing_else_is_expanded_and_a_missing_runtime_is_an_error() {
        let installed = installed();
        for (text, why) in [
            ("${HOME}/x", "unknown placeholder"),
            ("${runtime.python}/python", "not installed"),
            ("${pack/x", "unterminated"),
            ("${}", "unknown placeholder"),
        ] {
            let error = expand(text, &installed).unwrap_err();
            assert!(error.contains(why), "{text}: {error}");
        }
    }

    #[test]
    fn json_strings_are_expanded_where_they_are_without_breaking_the_json() {
        let installed = Installed {
            pack: Path::new("C:\\Users\\a b\\pack"),
            runtime: &runtimes,
        };
        let value =
            json!({ "tsserver": { "path": "${pack}/ts.js" }, "list": ["${pack}", 3, true, null] });
        let expanded = expand_json(&value, &installed).unwrap();
        assert_eq!(expanded["tsserver"]["path"], "C:\\Users\\a b\\pack/ts.js");
        assert_eq!(
            expanded["list"],
            json!(["C:\\Users\\a b\\pack", 3, true, null])
        );
        // The text of the result is valid JSON whatever the path holds.
        assert!(serde_json::from_str::<Value>(&expanded.to_string()).is_ok());
    }

    #[test]
    fn windows_commands_use_backslashes_and_get_the_exe_extension() {
        let command = program("C:/pack/bin/tool", true);
        assert_eq!(command, PathBuf::from("C:\\pack\\bin\\tool.exe"));
        // An explicit extension, and any command off Windows, is left alone.
        assert_eq!(program("C:/a/b.cmd", true), PathBuf::from("C:\\a\\b.cmd"));
        assert_eq!(program("/a/b/tool", false), PathBuf::from("/a/b/tool"));
    }
}
