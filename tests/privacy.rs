use std::{
    io::Write,
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use token_usage::{
    config::Config,
    model::{Provider, Snapshot},
    store::Store,
};

#[test]
fn network_access_is_opt_in_for_new_and_partial_settings() {
    assert!(!Config::default().online);
    assert!(!serde_json::from_str::<Config>("{}").unwrap().online);
    assert!(
        serde_json::from_str::<Config>(r#"{"online":true}"#)
            .unwrap()
            .online
    );
}

#[test]
fn offline_cli_does_not_fetch_missing_or_stale_prices() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = Config {
        online: true,
        ..Config::default()
    };
    std::fs::write(
        dir.path().join("settings.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let mut snapshot = Snapshot {
        refreshed_at: Some(chrono::Utc::now()),
        ..Snapshot::default()
    };
    for status in &mut snapshot.providers {
        status.detected = true;
    }
    Store::at(&dir.path().join("usage.db"))
        .unwrap()
        .save(&snapshot)
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let server = {
        let stop = stop.clone();
        let calls = calls.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        calls.fetch_add(1, Ordering::SeqCst);
                        let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("Proxy failed: {error}"),
                }
            }
        })
    };
    // Both a missing table and a stale table previously triggered a download.
    for cached in [false, true] {
        if cached {
            std::fs::write(dir.path().join("usage-model-rates.json"), r#"{"fetched_at_ms":1,"document":{"fixture-model":{"input_cost_per_token":1.0,"output_cost_per_token":2.0}}}"#).unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_usage"));
        for provider in Provider::ALL {
            command.env(
                format!("TOKEN_USAGE_{}_HOME", provider.key().to_ascii_uppercase()),
                dir.path().join(provider.key()),
            );
        }
        let output = command
            .args(["--offline", "--refresh", "--json"])
            .env("TOKEN_USAGE_DATA_DIR", dir.path())
            .env("HTTPS_PROXY", &proxy)
            .env("HTTP_PROXY", &proxy)
            .env("ALL_PROXY", &proxy)
            .env("NO_PROXY", "")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "Offline must not fetch model rates or check accounts"
    );
}
