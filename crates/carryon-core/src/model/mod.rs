//! Core domain model (spec §6): sessions, cuts, objects, actions, authority,
//! budgets. Pure types with no I/O.

pub mod action;
pub mod authority;
pub mod budget;
pub mod cut;
pub mod object;
pub mod session;

pub use action::{
    ActionAvailability, ActionDescriptor, ActionRequest, ActionResult, ActivationSpec,
    DependencyPlan, MeasurementSpec, OracleKind, OracleOutcome,
};
pub use authority::{AuthorityMode, AuthorityState};
pub use budget::{Budget, ThermalLimit};
pub use cut::{Cut, DerivedValidity};
pub use object::{Object, ObjectKind, Retention, Sensitivity};
pub use session::{Session, SessionState};
