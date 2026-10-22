mod common;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};
fn server(routes: Vec<(&'static str, Vec<u8>, u64)>) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let handle = std::thread::spawn(move || {
        for (headers, body, delay) in routes {
            let until = std::time::Instant::now() + Duration::from_secs(8);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(
                    std::time::Instant::now() < until,
                    "no HTTP request reached the fixture"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buffer = [0; 4096];
            let _ = stream.read(&mut buffer);
            std::thread::sleep(Duration::from_millis(delay));
            let _ = write!(
                stream,
                "HTTP/1.1 {headers}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    (url, handle)
}
fn download(app: &mut common::TestApp, url: String) {
    common::run(app, "file.openFromUrl", json!({}));
    app.harness.state_mut().core.url.input.clear();
    app.harness.get_by_label("Document URL").click();
    app.harness.run_steps(1);
    app.harness.event(egui::Event::Text(url));
    app.harness.run_steps(2);
    app.harness.get_by_label("Open").click();
    app.harness.run_steps(2);
    assert!(
        app.harness.state().core.url.cancellation.is_some(),
        "URL request did not start: {:?}",
        app.harness.state().core.url.error
    );
}
fn wait(app: &mut common::TestApp) {
    for _ in 0..200 {
        common::settle(app);
        if app.harness.state().core.url.cancellation.is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("download did not settle");
}
// spec 08: "Open a URL as an editable local copy"
#[test]
fn open_edit_save() {
    let (base, server) = server(vec![
        ("302 Found\r\nLocation: /table.csv", vec![], 0),
        (
            "200 OK\r\nContent-Type: text/csv",
            b"Name,Value\r\nAlice,old\r\n".to_vec(),
            0,
        ),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("download.csv");
    let dialogs = std::sync::Arc::new(notoir::files::ScriptedDialogs::default());
    dialogs.answer_save(Some(path.clone()));
    let mut app = common::start(|o| o.dialogs = Some(dialogs));
    download(&mut app, format!("{base}/redirect"));
    wait(&mut app);
    server.join().unwrap();
    let c = &app.harness.state().core;
    let doc = c.workbench.active_document().unwrap();
    let d = c.workbench.document(doc).unwrap();
    assert_eq!(c.document_title(doc), "table.csv");
    assert_eq!(
        d.source_url.as_deref(),
        Some(format!("{base}/table.csv").as_str())
    );
    assert!(d.path().is_none());
    assert!(d.is_dirty());
    assert_eq!(d.language.as_str(), "csv");
    common::run(&mut app, "csv.showPreview", json!({}));
    common::settle(&mut app);
    common::run(
        &mut app,
        "csv.action",
        json!({"op":"edit","row":1,"column":1}),
    );
    common::run(&mut app, "csv.action", json!({"op":"commit","value":"new"}));
    common::settle(&mut app);
    common::run(&mut app, "file.save", json!({}));
    common::settle(&mut app);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "Name,Value\r\nAlice,new\r\n"
    );
}
// spec 08: "Failed or cancelled URL downloads create no document"
#[test]
fn errors_and_cancellation() {
    let mut app = common::start(|_| {});
    for (header, body) in [
        ("404 Not Found", vec![]),
        ("200 OK\r\nContent-Type: application/pdf", b"%PDF".to_vec()),
        ("200 OK\r\nContent-Type: text/plain", vec![0, 1, 2]),
    ] {
        let (url, handle) = server(vec![(header, body, 0)]);
        download(&mut app, url);
        wait(&mut app);
        handle.join().unwrap();
        assert!(app.harness.state().core.url.error.is_some());
        assert!(app.harness.state().core.workbench.documents.is_empty());
        common::run(&mut app, "file.cancelUrl", json!({}));
    }
    let (url, handle) = server(vec![("302 Found\r\nLocation: /loop", vec![], 0); 6]);
    download(&mut app, url);
    wait(&mut app);
    handle.join().unwrap();
    assert!(app.harness.state().core.url.error.is_some());
    assert!(app.harness.state().core.workbench.documents.is_empty());
    common::run(&mut app, "file.cancelUrl", json!({}));
    let (url, handle) = server(vec![(
        "200 OK\r\nContent-Type: text/plain",
        b"late".to_vec(),
        150,
    )]);
    download(&mut app, url);
    common::run(&mut app, "file.cancelUrl", json!({}));
    handle.join().unwrap();
    common::settle(&mut app);
    assert!(app.harness.state().core.workbench.documents.is_empty());
    assert!(!app.harness.state().core.url.open);
    common::run(&mut app, "file.openFromUrl", json!({}));
    app.harness
        .state_mut()
        .core
        .url_start("file:///tmp/a".into());
    assert!(app.harness.state().core.url.error.is_some());
}
