//! S2 resource descriptions, instructions and session state for EMS assets.
//!
//! A Customer Energy Manager selects a control type to schedule a resource.
//! [`describe_site`] builds the descriptions and lists assets it could not
//! represent. [`Session`] handles protocol negotiation and timed instructions.
//! [`instruct`] translates accepted instructions into local commands.
//!
//! The protocol types are provided by [`s2_kit`]. See the
//! [flexibility guide](https://hupe1980.github.io/ems/docs/flexibility/)
//! for the battery, hot-water, EV, PV and appliance mappings.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]

pub mod describe;
pub mod instruct;
pub mod map;
pub mod session;
pub mod site;

pub use describe::{
    BatteryDescription, DhwDescription, EvDescription, EvStorage, HeatPumpDescription,
    ProgrammeDescription, describe_battery, describe_dhw, describe_ev, describe_evse,
    describe_heat_pump, describe_programme, describe_pv, resource_manager_details,
};
pub use instruct::{
    Direction, InstructError, actuator_factor, battery_power, dhw_power, envelope_command,
    envelope_interval, envelope_now, heat_pump_state, programme_start,
};
pub use map::{ControlType, control_type_for, roles_for};
pub use session::{
    CloseReason, Instructed, Offer, Ratings, Report, Session, SessionEvent, SessionState,
    profile_status,
};
pub use site::{DescribeContext, SiteDescription, describe_site, sessions_for};
