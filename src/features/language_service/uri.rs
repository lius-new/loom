//! `file:` URIs for documents, in both directions.
//!
//! Servers name files by URI, so a path must survive the round trip exactly,
//! including spaces, non-ASCII names, `#`, `?` and `%`, Windows drive letters
//! and UNC shares. The logic works on strings with the path style given
//! explicitly, so the Windows rules are tested on every platform.
use std::path::{Path, PathBuf};

/// Bytes kept as they are inside a path segment. Everything else is %-encoded.
fn unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn encode_segment(segment: &str, out: &mut String) {
    for &byte in segment.as_bytes() {
        if unreserved(byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
}

fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = text.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The URI of an absolute path written in the given style. `None` for a relative
/// path, which names no single file.
pub fn file_uri_for(path: &str, windows: bool) -> Option<String> {
    let mut uri = String::from("file://");
    if windows {
        let normalized = path.replace('\\', "/");
        if let Some(unc) = normalized.strip_prefix("//") {
            // \\server\share\a.rs: the server is the authority.
            let (host, rest) = unc.split_once('/')?;
            if host.is_empty() {
                return None;
            }
            uri.push_str(host);
            for segment in rest.split('/') {
                uri.push('/');
                encode_segment(segment, &mut uri);
            }
            return Some(uri);
        }
        let bytes = normalized.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/';
        if !drive {
            return None;
        }
        uri.push('/');
        uri.push(bytes[0] as char);
        uri.push(':');
        for segment in normalized[2..].split('/').skip(1) {
            uri.push('/');
            encode_segment(segment, &mut uri);
        }
        Some(uri)
    } else {
        let rest = path.strip_prefix('/')?;
        for segment in rest.split('/') {
            uri.push('/');
            encode_segment(segment, &mut uri);
        }
        Some(uri)
    }
}

/// The path a `file:` URI names, in the given style.
pub fn file_path_for(uri: &str, windows: bool) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    let path = decode(path)?;
    if windows {
        if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
            return Some(format!("\\\\{authority}{}", path.replace('/', "\\")));
        }
        // /C:/dir/a.rs, with the colon possibly encoded (already decoded above).
        let bytes = path.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':';
        if !drive {
            return None;
        }
        return Some(path[1..].replace('/', "\\"));
    }
    if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
        return None;
    }
    path.starts_with('/').then_some(path)
}

/// The URI of an absolute path on this platform.
pub fn file_uri(path: &Path) -> Option<String> {
    file_uri_for(path.to_str()?, cfg!(windows))
}

/// The path a `file:` URI names on this platform.
pub fn file_path(uri: &str) -> Option<PathBuf> {
    file_path_for(uri, cfg!(windows)).map(PathBuf::from)
}

/// A string that is equal for two spellings of the same file's URI: servers differ
/// in how they write the drive letter and the colon, and Windows paths ignore case.
pub fn key(uri: &str) -> String {
    match file_path_for(uri, cfg!(windows)) {
        Some(path) if cfg!(windows) => path.to_lowercase(),
        Some(path) => path,
        None => uri.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix(path: &str) -> String {
        file_uri_for(path, false).unwrap()
    }
    fn windows(path: &str) -> String {
        file_uri_for(path, true).unwrap()
    }

    #[test]
    fn unix_paths_become_uris_and_back() {
        assert_eq!(
            unix("/home/user/src/main.rs"),
            "file:///home/user/src/main.rs"
        );
        assert_eq!(unix("/"), "file:///");
        assert_eq!(
            file_path_for("file:///home/user/src/main.rs", false).unwrap(),
            "/home/user/src/main.rs"
        );
    }

    #[test]
    fn awkward_characters_survive_the_round_trip() {
        let paths = [
            "/work/my project/a b.rs",
            "/work/中文/文件 😀.rs",
            "/work/100%/a#b?c.rs",
            "/work/semi;colon&amp=eq+plus(paren)!.rs",
            "/work/back\\slash.rs",
            "/work/tilde~dot.name_x-y.rs",
        ];
        for path in paths {
            let uri = unix(path);
            assert!(uri.is_ascii(), "{uri}");
            assert!(
                !uri.contains(' ') && !uri[7..].contains(['#', '?']),
                "{uri}"
            );
            assert_eq!(file_path_for(&uri, false).as_deref(), Some(path), "{uri}");
        }
        assert_eq!(unix("/a b/中.rs"), "file:///a%20b/%E4%B8%AD.rs");
        assert_eq!(unix("/50%/x#y"), "file:///50%25/x%23y");
    }

    #[test]
    fn windows_drive_paths_use_forward_slashes_and_keep_the_colon() {
        assert_eq!(
            windows(r"C:\Users\me\src\main.rs"),
            "file:///C:/Users/me/src/main.rs"
        );
        assert_eq!(
            windows(r"d:\My Docs\中文.rs"),
            "file:///d:/My%20Docs/%E4%B8%AD%E6%96%87.rs"
        );
        assert_eq!(
            windows("C:/already/slashes.rs"),
            "file:///C:/already/slashes.rs"
        );
        assert_eq!(
            file_path_for("file:///C:/Users/me/src/main.rs", true).as_deref(),
            Some(r"C:\Users\me\src\main.rs")
        );
        assert_eq!(
            file_path_for("file:///d:/My%20Docs/%E4%B8%AD%E6%96%87.rs", true).as_deref(),
            Some(r"d:\My Docs\中文.rs")
        );
    }

    #[test]
    fn servers_that_encode_the_drive_colon_are_understood() {
        // rust-analyzer and VS Code write file:///c%3A/Users/...
        assert_eq!(
            file_path_for("file:///c%3A/Users/me/a.rs", true).as_deref(),
            Some(r"c:\Users\me\a.rs")
        );
        assert_eq!(
            file_path_for("file:///C%3a/Users/me/a.rs", true).as_deref(),
            Some(r"C:\Users\me\a.rs"),
            "lowercase hex too"
        );
    }

    #[test]
    fn unc_shares_use_the_server_as_the_authority() {
        assert_eq!(
            windows(r"\\fileserver\share\dir\a b.rs"),
            "file://fileserver/share/dir/a%20b.rs"
        );
        assert_eq!(
            file_path_for("file://fileserver/share/dir/a%20b.rs", true).as_deref(),
            Some(r"\\fileserver\share\dir\a b.rs")
        );
        assert_eq!(file_uri_for(r"\\onlyhost", true), None);
        assert_eq!(file_uri_for(r"\\\bad", true), None);
        // "localhost" is the local machine, not a share.
        assert_eq!(
            file_path_for("file://localhost/C:/a.rs", true).as_deref(),
            Some(r"C:\a.rs")
        );
    }

    #[test]
    fn things_that_are_not_absolute_file_paths_are_refused() {
        assert_eq!(file_uri_for("relative/a.rs", false), None);
        assert_eq!(file_uri_for("", false), None);
        assert_eq!(file_uri_for(r"relative\a.rs", true), None);
        assert_eq!(file_uri_for("C:relative.rs", true), None);
        for uri in [
            "http://example.com/a.rs",
            "file:/no/authority.rs",
            "untitled:Untitled-1",
            "file:///bad%zzescape",
            "file:///truncated%4",
            "file:///not-utf8%FF%FE",
            "",
        ] {
            assert_eq!(file_path_for(uri, false), None, "{uri}");
        }
        assert_eq!(
            file_path_for("file://otherhost/share/a.rs", false),
            None,
            "a remote host on Unix"
        );
        assert_eq!(file_path_for("file:///no-drive/a.rs", true), None);
    }

    #[test]
    fn two_spellings_of_one_windows_file_share_a_key() {
        if !cfg!(windows) {
            return;
        }
        let ours = file_uri(Path::new(r"C:\Users\Me\src\a.rs")).unwrap();
        let servers = "file:///c%3A/users/me/SRC/a.rs";
        assert_eq!(key(&ours), key(servers));
        assert_ne!(key(&ours), key("file:///c%3A/users/me/src/b.rs"));
    }

    #[test]
    fn a_non_file_uri_is_its_own_key() {
        assert_eq!(key("untitled:Untitled-1"), "untitled:Untitled-1");
    }

    #[test]
    fn this_platform_round_trips_a_real_path() {
        let path = std::env::temp_dir().join("a dir").join("文件.rs");
        let uri = file_uri(&path).expect("the temp dir is absolute");
        assert_eq!(file_path(&uri).as_deref(), Some(path.as_path()));
    }
}
