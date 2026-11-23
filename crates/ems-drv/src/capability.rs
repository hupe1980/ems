//! What a driver has declared it can do.

/// Measurements, limits and commands supported by one driver. Checked at registration.
#[expect(
    clippy::struct_excessive_bools,
    reason = "a capability set is a handful of independent yes/no facts about one \
              driver; folding them into a state machine would relate things that \
              are not related"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DriverCapabilities {
    /// Periodic readings are expected unless `reports_on_change` is set. A grid driver may supply optional MGCP measurements while carrying the operator’s instruction `[A1 4.6]`.
    pub measures: bool,
    /// It can be told a power setpoint or ceiling.
    pub accepts_commands: bool,
    /// Can select heating or cooling. A reversible unit requires a driver that can select its direction.
    pub sets_thermal_mode: bool,
    /// Carries the network operator’s connection limits.
    pub reports_grid_limits: bool,
    /// Measurements update on change. Judge silence by link state unless the peer supplies a measurement timestamp. Periodic heartbeats retain their own deadline.
    pub reports_on_change: bool,
    /// Reports available generation before curtailment, so the guard can distinguish potential output from measured output.
    pub reports_available_power: bool,
}

impl DriverCapabilities {
    /// The same, and able to turn a reversible thermal device round.
    ///
    /// A builder rather than a sixth constructor: the direction is orthogonal to
    /// every other question here — a unit can be measured or not, polled or
    /// notified, and still reversible — and a constructor per combination is the
    /// matrix this struct exists to avoid.
    #[must_use]
    pub const fn setting_thermal_mode(mut self) -> Self {
        self.sets_thermal_mode = true;
        self
    }

    /// A driver that reports a device and takes commands.
    #[must_use]
    pub const fn device() -> Self {
        Self {
            measures: true,
            accepts_commands: true,
            reports_grid_limits: false,
            reports_on_change: false,
            reports_available_power: false,
            sets_thermal_mode: false,
        }
    }

    /// A driver that only listens — a meter.
    #[must_use]
    pub const fn meter() -> Self {
        Self {
            measures: true,
            accepts_commands: false,
            reports_grid_limits: false,
            reports_on_change: false,
            reports_available_power: false,
            sets_thermal_mode: false,
        }
    }

    /// A driver that takes commands and measures nothing.
    ///
    /// The shape of an EEBUS use case that *drives* a device whose consumption
    /// somebody else meters — a compressor over OHPCF beside the site's own
    /// meter. Claiming `measures` here would have two drivers reporting one
    /// asset, which the registry refuses at start-up and is right to.
    #[must_use]
    pub const fn commanding() -> Self {
        Self {
            measures: false,
            accepts_commands: true,
            reports_grid_limits: false,
            reports_on_change: false,
            reports_available_power: false,
            sets_thermal_mode: false,
        }
    }

    /// A driver that watches a device without metering or driving it.
    ///
    /// The shape of a use case that reports a *session fact* rather than a
    /// quantity: EEBUS EVCC and EVSOC say whether there is a car on the cable
    /// and how full it is, and never what the wallbox is drawing — the wallbox's
    /// own Modbus driver does that, and commands it.
    ///
    /// `measures` is false and that is the point rather than modesty. The flag
    /// is what **silence is judged by**, so a driver that claimed it and then
    /// only ever reported a car arriving would be counted as a device nobody can
    /// hear for the whole of every day the car is away.
    #[must_use]
    pub const fn observing() -> Self {
        Self {
            measures: false,
            accepts_commands: false,
            reports_grid_limits: false,
            reports_on_change: false,
            reports_available_power: false,
            sets_thermal_mode: false,
        }
    }

    /// A driver that carries the network operator's limits and nothing else.
    ///
    /// It accepts no commands, and that is the regulation rather than an
    /// omission: a household does not command its own reduction.
    #[must_use]
    pub const fn grid() -> Self {
        Self {
            measures: false,
            accepts_commands: false,
            reports_grid_limits: true,
            reports_on_change: false,
            reports_available_power: false,
            sets_thermal_mode: false,
        }
    }

    /// The same, and its values arrive on change rather than on a cadence.
    ///
    /// For a driver none of whose use cases put a function on a clock — EEBUS's
    /// measurement family — where silence is the protocol working rather than a
    /// device that has gone away.
    #[must_use]
    pub const fn on_change(mut self) -> Self {
        self.reports_on_change = true;
        self
    }

    /// The same, and it publishes available power.
    #[must_use]
    pub const fn with_available_power(mut self) -> Self {
        self.reports_available_power = true;
        self
    }
}
