use super::*;
#[test]
fn example_uses_a_fixed_confidential_origin() {
    let mut config: Settings = toml::from_str(include_str!("../portald.example.toml")).unwrap();
    assert_eq!(
        Collector::new(&config).unwrap().receipt.path(),
        "/v1/me/departure"
    );
    for origin in [
        "http://collector.example",
        "https://collector.example/path",
        "https://user@collector.example",
        "https://collector.example/?site=foreign",
    ] {
        config.collector_origin = origin.into();
        assert!(Collector::new(&config).is_err());
    }
}
#[tokio::test]
async fn redirect_is_refused_and_credential_is_confined() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let checked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let forwarded = checked.clone();
    let app = Router::new()
        .route(
            "/v1/me/departure",
            get(move |headers: HeaderMap| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer sample");
                assert!(!headers.contains_key(header::COOKIE));
                forwarded.store(true, std::sync::atomic::Ordering::SeqCst);
                axum::response::Redirect::temporary("/other")
            }),
        )
        .route("/other", get(|| async { "{}" }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let collector = Collector::new(&Settings {
        collector_origin: format!("http://{addr}"),
        ..Default::default()
    })
    .unwrap();
    let mut headers = HeaderMap::new();
    assert_eq!(
        receipt(State(collector.clone()), headers.clone())
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer sample"),
    );
    headers.insert(header::COOKIE, HeaderValue::from_static("private=1"));
    let response = receipt(State(collector), headers).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert!(checked.load(std::sync::atomic::Ordering::SeqCst));
    server.abort();
}
