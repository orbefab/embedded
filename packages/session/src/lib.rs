//! The live commands as one typed table, and the session loop that runs them on one engine
//! session. `orbe test` runs it in-process, and a per-session background process runs it for
//! the live commands.

mod command;
mod condition;
mod duration;
mod live;
mod reply;
mod target;
pub mod test;
#[cfg(test)]
mod typescript;
pub mod viewer;

pub use command::{COMMANDS, Command, Spec, Step, UNTIL_TIMEOUT, WorldSet, spec};
pub use condition::{Amount, Condition, Needle, Op, Rgb, State as Want, Unit};
pub use duration::Duration;
pub use live::{
    BRIGHTNESS_WINDOW, CHANGES, Channels, FrameState, Live, Pace, Progress, SERIAL_LINES, Sample,
};
pub use orbe_scene::Files;
pub use reply::{
    Change, Check, Code, Colors, Data, Diagnostic, Error, Finding, Got, LedRead, Level, Light, Met,
    PartState, Reading, Reply, Snapshot, State, Stepped,
};
pub use target::{Ref, Target, resolve};
