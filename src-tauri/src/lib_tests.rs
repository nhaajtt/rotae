//! Tests for the small commands in lib.rs.

#[test]
fn only_known_links_open() {
    let ok = super::link_allowed;
    assert!(ok("https://github.com/nhaajtt/rotae"));
    assert!(ok("https://github.com/nhaajtt/rotae/issues/new?title=Bug%20report&body=a%0Ab"));
    assert!(ok("https://www.libreoffice.org/download/download-libreoffice/"));
    assert!(ok("https://www.microsoft.com/en-us/microsoft-365"));
    for bad in [
        "https://github.com/nhaajtt/rotae/../../evil/repo",
        "https://github.com/nhaajtt/rotae/%2e%2e/%2E%2E/evil",
        "https://github.com/nhaajtt/rotae/..%2fevil",
        "https://github.com/nhaajtt/rotae-evil",
        "https://github.com/nhaajtt/rotae/./x/../../y",
        "https://github.com/nhaajtt/rotae/x\\..\\y",
        "https://www.microsoft.com@evil.example/",
        "https://www.libreoffice.org/ x",
        "https://www.libreoffice.org/\"x",
        "http://github.com/nhaajtt/rotae",
        "file:///C:/Windows/System32/calc.exe",
        "https://evil.example/https://github.com/nhaajtt/rotae/",
    ] {
        assert!(!ok(bad), "{bad}");
    }
}
