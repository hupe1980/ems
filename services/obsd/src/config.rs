//! What `obsd` is told.

/// Everything `obsd` is configured with.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    /// Shared daemon settings under `[service]` (D160).
    #[serde(default)]
    pub service: ems_service::Settings,
    /// Retention window for each site's reports, in days (D157). Defaults to 60.
    pub keep_days: usize,
    /// PostgreSQL connection for household reports and compliance findings (D157).
    #[serde(default)]
    pub database: ems_service::DbSettings,
    /// Seconds since the last report before a site is marked silent. Defaults to two days.
    pub silent_after_s: u64,
    /// Signing keys indexed by household. Each key must belong to one site (D114).
    /// Configure both keys during rotation. An empty map disables ingestion.
    pub webhook_secrets: std::collections::BTreeMap<String, Vec<ems_service::Secret>>,
    /// Households visible to each tenant. An operator with tenant `*` can read every site (D112).
    #[serde(default)]
    pub tenants: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    /// Site-specific tokens accepted only by the customer receipt endpoint.
    #[serde(default)]
    pub customer_tokens: std::collections::BTreeMap<String, ems_service::Secret>,
    /// The operator credentials, each scoped to a tenant.
    #[serde(default)]
    pub operators: Vec<ems_service::OperatorCredential>,
    /// The Model Context Protocol surface, off by default.
    ///
    /// It holds a household's data, so a token is **required** when it is
    /// switched on and the surface answers as whatever authority that token
    /// already carries here — an operator, or one site.
    #[serde(default)]
    pub mcp: ems_service::McpSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            service: ems_service::Settings::default(),
            keep_days: 60,
            database: ems_service::DbSettings::default(),
            silent_after_s: 2 * 24 * 3600,
            webhook_secrets: std::collections::BTreeMap::new(),
            tenants: std::collections::BTreeMap::new(),
            operators: Vec::new(),
            customer_tokens: std::collections::BTreeMap::new(),
            mcp: ems_service::McpSettings::default(),
        }
    }
}

impl AsMut<ems_service::Settings> for Settings {
    fn as_mut(&mut self) -> &mut ems_service::Settings {
        &mut self.service
    }
}

#[cfg(test)]
mod example_tests {
    use super::*;

    /// The example file that ships with the daemon.
    ///
    /// Parsed by a test rather than trusted: a commented example that has
    /// drifted from the struct it documents is worse than none, because it is
    /// read by whoever is deploying this and every line of it looks
    /// authoritative. `include_str!` makes it a build input.
    const EXAMPLE: &str = include_str!("../obsd.example.toml");

    #[test]
    fn the_example_configuration_parses_and_describes_a_service() {
        let settings: Settings = toml::from_str(EXAMPLE).expect("the shipped example parses");
        assert!(
            !settings.operators.is_empty(),
            "an example with no operator would document a service nobody can read"
        );
        assert!(
            settings.database.tls,
            "what crosses this socket is a fleet's household data"
        );
        assert!(
            settings.database.statement_timeout_s > 0,
            "a serving replica needs a bound on a query whose reader has gone"
        );
    }
}
