use notoir_platform::associations::EXTENSIONS;

#[test]
fn package_declarations_cover_the_checkbox_catalog() {
    let metadata = include_str!("../../../apps/notoir/Cargo.toml");
    let manifest = include_str!("../../../packaging/windows-store/AppxManifest.xml");
    for extension in EXTENSIONS {
        assert!(
            metadata.contains(&format!("\"{extension}\"")),
            "Missing packaged extension: {extension}"
        );
        assert!(
            manifest.contains(&format!("<uap:FileType>.{extension}</uap:FileType>")),
            "Missing MSIX extension: {extension}"
        );
    }
    let installer = include_str!("../../../scripts/templates/windows-installer.nsi");
    assert!(!installer.contains("!insertmacro APP_ASSOCIATE "));
    assert!(installer.contains("OpenWithProgids"));
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn native_inspection_reports_effective_defaults_without_mutating_them() {
    use notoir_platform::associations::{Backend, NativeBackend};
    let backend = NativeBackend::new().unwrap();
    let rows = backend.inspect().unwrap();
    assert_eq!(rows.len(), EXTENSIONS.len());
    for extension in EXTENSIONS {
        let row = rows.iter().find(|r| r.extension == *extension).unwrap();
        assert!(!row.content_type.is_empty());
        if row.is_notoir {
            assert!(row.handler.is_some());
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_adapter_roundtrip_preserves_other_mime_defaults() {
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let data = root.path().join("data");
    let bin = root.path().join("bin");
    for path in [
        &config,
        &data.join("mime"),
        &data.join("applications"),
        &bin,
    ] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(
        data.join("mime/globs2"),
        "50:text/plain:*.txt\n50:text/plain:*.log\n50:text/markdown:*.md\n",
    )
    .unwrap();
    fs::write(
        data.join("applications/old.desktop"),
        "[Desktop Entry]\nName=Previous\nType=Application\nExec=previous %F\n",
    )
    .unwrap();
    fs::write(
        config.join("mimeapps.list"),
        "# preserve\n[Default Applications]\ntext/plain=old.desktop;\nimage/png=viewer.desktop;\n",
    )
    .unwrap();
    let query = bin.join("xdg-mime");
    fs::write(&query,r#"#!/bin/sh
awk -F= -v mime="$3" '$0=="[Default Applications]" {section=1;next} /^\[/ {section=0} section && $1==mime {split($2, ids, ";"); print ids[1]; exit}' "$XDG_CONFIG_HOME/mimeapps.list"
"#).unwrap();
    fs::set_permissions(&query, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "linux_fixture_worker",
            "--ignored",
            "--nocapture",
        ])
        .env("NOTOIR_ASSOCIATION_FIXTURE", root.path())
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_DATA_DIRS", root.path().join("empty"))
        .env("XDG_CURRENT_DESKTOP", "")
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(config.join("mimeapps.list"))
            .unwrap()
            .contains("image/png=viewer.desktop;")
    );
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "internal subprocess with isolated XDG directories; exercised by linux_adapter_roundtrip_preserves_other_mime_defaults"]
fn linux_fixture_worker() {
    use notoir_platform::associations::{self, Backend, NativeBackend, Selection};
    let root = std::path::PathBuf::from(
        std::env::var_os("NOTOIR_ASSOCIATION_FIXTURE").expect("isolated fixture required"),
    );
    let backend = NativeBackend::new().unwrap();
    let journal = root.join("journal.json");
    let rows = backend.inspect().unwrap();
    assert_eq!(
        rows.iter()
            .find(|r| r.extension == "txt")
            .unwrap()
            .handler
            .as_deref(),
        Some("old.desktop")
    );
    assert_eq!(
        rows.iter()
            .find(|r| r.extension == "txt")
            .unwrap()
            .content_type,
        rows.iter()
            .find(|r| r.extension == "log")
            .unwrap()
            .content_type
    );
    let selected = Selection {
        content_type: "text/plain".into(),
        expected_handler: Some("old.desktop".into()),
        selected: true,
    };
    let result = associations::apply(&backend, &journal, &[selected]).unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(
        result
            .rows
            .iter()
            .filter(|r| r.content_type == "text/plain")
            .all(|r| r.is_notoir)
    );
    let selected = Selection {
        content_type: "text/plain".into(),
        expected_handler: Some(backend.identity().into()),
        selected: false,
    };
    let result = associations::apply(&backend, &journal, &[selected]).unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert!(
        result
            .rows
            .iter()
            .filter(|r| r.content_type == "text/plain")
            .all(|r| r.handler.as_deref() == Some("old.desktop"))
    );
    let selection = Selection {
        content_type: "text/plain".into(),
        expected_handler: Some("old.desktop".into()),
        selected: true,
    };
    associations::apply(&backend, &journal, &[selection]).unwrap();
    std::fs::remove_file(root.join("data/applications/old.desktop")).unwrap();
    let selection = Selection {
        content_type: "text/plain".into(),
        expected_handler: Some(backend.identity().into()),
        selected: false,
    };
    let result = associations::apply(&backend, &journal, &[selection]).unwrap();
    assert!(result.pending);
    assert!(!result.errors.is_empty());
}
