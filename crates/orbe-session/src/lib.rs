//! The live commands as one typed table, and the session loop that runs them on one engine
//! session. `orbe test` runs it in-process. A per-session background process will run the
//! same loop for the live commands.

mod command;
mod condition;
mod duration;
mod live;
mod reply;
mod target;
pub mod test;

pub use command::{COMMANDS, Command, Spec, Step, UNTIL_TIMEOUT, spec};
pub use condition::{Amount, Condition, Op, State as Want, Unit};
pub use duration::Duration;
pub use live::Live;
pub use reply::{
    Change, Check, Code, Data, Diagnostic, Error, Got, Level, PartState, Reading, Reply, Snapshot,
    State, Stepped,
};
pub use target::{Ref, Target, resolve};
