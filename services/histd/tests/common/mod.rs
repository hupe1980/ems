pub fn database(pg: &ems_service::testdb::Postgres) -> ems_service::DbSettings {
    let admin = std::env::var("EMS_TEST_POSTGRES")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:55432".into());
    let mut url = url::Url::parse(&admin).unwrap();
    url.set_path(&pg.name);
    ems_service::DbSettings {
        url: ems_service::Secret::literal(url.as_str()),
        tls: false,
        ..Default::default()
    }
}
