//! The live commands as one typed table, and the session loop that runs them on one engine
//! session. `orbe test` runs it in-process, and a per-session background process runs it for
//! the live commands.

mod command;
mod condition;
mod digit;
mod duration;
mod live;
mod reply;
mod target;
pub mod test;
#[cfg(test)]
mod typescript;
pub mod viewer;

/// A command failed.
pub const EXIT_FAILED: u8 = 1;
/// The command line is not one this orbe runs.
pub const EXIT_USAGE: u8 = 2;
/// A file or a socket could not be read or written.
pub const EXIT_IO: u8 = 3;
/// The session's process is gone.
pub const EXIT_LOST: u8 = 4;

pub use command::{COMMANDS, Command, ParamSet, Spec, Step, UNTIL_TIMEOUT, WorldSet, spec};
pub use condition::{Amount, Condition, Needle, Op, Rgb, State as Want, Unit};
pub use duration::Duration;
pub use live::{
    BRIGHTNESS_WINDOW, CHANGES, Channels, FrameState, Glass, Live, Pace, Progress, SERIAL_LINES,
    STEP_BUDGET, Sample, StepTimeoutHint, test_step_budget,
};
pub use orbe_scene::Files;
pub use reply::{
    Change, Check, Code, Colors, Data, Diagnostic, Error, Finding, Got, LedRead, Level, Light, Met,
    PartState, Reading, Reply, Screen, Snapshot, State, Stepped,
};
pub use target::{Ref, Target, resolve};
