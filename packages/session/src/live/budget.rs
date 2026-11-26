//! Wall-time budget for `step`, shared by live sessions and `orbe test`.
//!
//! A `step` stops once its elapsed wall time reaches the budget, reporting the simulated time
//! reached, the duration asked for, the speed relative to real time, and a hint matching where
//! it ran.

use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use crate::command::{Command, Step};
use crate::duration::Duration as SimDuration;
use crate::live::{Live, Progress};
use crate::reply::{Code, Reply};

/// Wall-time budget for one `step`, well under the 2-minute command limit agent harnesses
/// commonly use, so a slow scene stops and reports cleanly instead of hanging the caller.
pub const STEP_BUDGET: Duration = Duration::from_secs(30);

/// Where a `step` runs, which decides the hint `step_timeout` gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepTimeoutHint {
    /// A live session that stays paused at the reached time.
    Session,
    /// An `orbe test` run, whose session ends with the failed test.
    Test,
}

impl Live {
    /// Runs `command` with a wall-time `budget` on `step`. `during` runs after each slice of a
    /// `step` with the slice's progress and elapsed wall time; returning `Break` stops the step
    /// as `cancelled` before checking `budget`. When `elapsed >= budget` while the step is still
    /// running, the step stops with `step_timeout`, reporting the reached time, the asked time,
    /// the speed relative to real time, and a hint matching `hint`.
    pub fn run_with_budget(
        &mut self,
        command: &Command,
        budget: Duration,
        hint: StepTimeoutHint,
        during: &mut dyn FnMut(&Progress, Duration) -> ControlFlow<()>,
    ) -> Reply {
        let started = matches!(command, Command::Step(_)).then(wall);
        let mut timed_out: Option<(u64, u64, Duration)> = None;
        let mut reply = self.run_with(command, &mut |progress| {
            let elapsed = started.map_or(Duration::ZERO, |at| wall().duration_since(at));
            if during(progress, elapsed).is_break() {
                return ControlFlow::Break(());
            }
            if started.is_some() && elapsed >= budget {
                timed_out = Some((progress.from_ps, progress.end_ps, elapsed));
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        if let (Command::Step(step), Some((from_ps, end_ps, elapsed))) = (command, timed_out) {
            say_step_timeout(&mut reply, step, from_ps, end_ps, budget, elapsed, hint);
        }
        reply
    }
}

/// Parses a debug-only `ORBE_TEST_STEP_BUDGET` override, falling back to [`STEP_BUDGET`].
/// A release build ignores it.
pub fn test_step_budget(text: Option<&str>) -> Duration {
    #[cfg(not(debug_assertions))]
    {
        let _ = text;
        STEP_BUDGET
    }
    #[cfg(debug_assertions)]
    {
        text.and_then(|value| SimDuration::parse(value).ok())
            .map(|duration| Duration::from_nanos(duration.as_ps() / 1000))
            .filter(|duration| !duration.is_zero())
            .unwrap_or(STEP_BUDGET)
    }
}

/// Turns a `cancelled` step reply into `step_timeout` with the reached time, asked time, speed
/// relative to real time, and a hint for the caller's context.
fn say_step_timeout(
    reply: &mut Reply,
    step: &Step,
    from_ps: u64,
    end_ps: u64,
    budget: Duration,
    elapsed: Duration,
    hint: StepTimeoutHint,
) {
    let Some(error) = &mut reply.error else {
        return;
    };
    if error.code != Code::Cancelled {
        return;
    }
    let reached_ps = reply.t_ps;
    let advanced_ps = reached_ps.saturating_sub(from_ps);
    let remaining_ps = end_ps.saturating_sub(reached_ps);
    let reached = SimDuration::from_ps(reached_ps);
    let remaining = SimDuration::from_ps(remaining_ps);
    let budget_ps = u64::try_from(budget.as_nanos())
        .unwrap_or(u64::MAX)
        .saturating_mul(1000);
    let budget_shown = SimDuration::from_ps(budget_ps);
    let pace = format_pace(elapsed, advanced_ps);
    let pace_note = if pace.is_empty() {
        String::new()
    } else {
        format!(" ({pace})")
    };
    error.code = Code::StepTimeout;
    match step {
        Step::For(asked) => {
            let asked_part = if from_ps == 0 {
                format!("asked for {asked}")
            } else {
                format!("{} of the {asked} asked", SimDuration::from_ps(advanced_ps))
            };
            error.message = format!(
                "step stopped at t={reached} ({asked_part}) after the {budget_shown} wall-time limit{pace_note}"
            );
            error.hint = match hint {
                StepTimeoutHint::Session => format!(
                    "the session is paused at t={reached}; run `orbe step {remaining}` to step the rest"
                ),
                StepTimeoutHint::Test => {
                    "run `orbe test --step-timeout <duration>` for a longer limit, or shorten the step"
                        .into()
                }
            };
        }
        Step::Until { condition, timeout } => {
            let asked_part = if from_ps == 0 {
                format!("timeout {timeout}")
            } else {
                format!(
                    "{} of the {timeout} timeout",
                    SimDuration::from_ps(advanced_ps)
                )
            };
            let cond = shell_condition(&condition.to_string());
            error.message = format!(
                "step --until stopped at t={reached} ({asked_part}) after the {budget_shown} wall-time limit{pace_note}"
            );
            error.hint = match hint {
                StepTimeoutHint::Session => format!(
                    "the condition was not met yet and the session is paused at t={reached}; run `orbe step --until {cond} --timeout {remaining}` to keep waiting"
                ),
                StepTimeoutHint::Test => {
                    "the condition was not met yet; run `orbe test --step-timeout <duration>` for a longer limit, or check the circuit"
                        .into()
                }
            };
        }
    }
}

/// How fast the scene simulated over one step relative to real time.
fn format_pace(elapsed: Duration, sim_ps: u64) -> String {
    let wall_secs = elapsed.as_secs_f64();
    let sim_secs = sim_ps as f64 / 1e12;
    if sim_secs <= 0.0 || wall_secs <= 0.0 {
        return String::new();
    }
    if wall_secs >= sim_secs {
        format!(
            "{}x slower than real time",
            format_factor(wall_secs / sim_secs)
        )
    } else {
        let faster = sim_secs / wall_secs;
        if (faster * 10.0).round() <= 10.0 {
            "at 1x real time".into()
        } else {
            format!("{}x faster than real time", format_factor(faster))
        }
    }
}

/// Rounds a speed ratio to a whole number (at 10x and up, or exact whole numbers) or one decimal.
fn format_factor(ratio: f64) -> String {
    let tenths = (ratio * 10.0).round() as u64;
    if tenths >= 100 {
        format!("{}", ratio.round() as u64)
    } else if tenths.is_multiple_of(10) {
        format!("{}", tenths / 10)
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

/// Quotes a condition for a shell when it holds characters a shell interprets, such as `>`, `(`
/// or `"`. Plain conditions such as `@led1 lit` stay unquoted.
fn shell_condition(text: &str) -> String {
    let needs_quotes = text.bytes().any(|b| {
        matches!(
            b,
            b'"' | b'\''
                | b'<'
                | b'>'
                | b'('
                | b')'
                | b';'
                | b'&'
                | b'|'
                | b'$'
                | b'`'
                | b'\\'
                | b'*'
                | b'?'
                | b'['
                | b']'
                | b'#'
                | b'~'
                | b'!'
        )
    });
    if !needs_quotes {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "wall-time step budget stops a slow step instead of hanging"
)]
fn wall() -> Instant {
    Instant::now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reply::{Data, Error, State};

    #[test]
    fn pace_rounds_to_whole_or_one_decimal() {
        assert_eq!(
            format_pace(Duration::from_secs(30), 937_500_000_000),
            "32x slower than real time"
        );
        assert_eq!(
            format_pace(Duration::from_secs(30), 1_157_000_000_000),
            "26x slower than real time"
        );
        assert_eq!(
            format_pace(Duration::from_secs(30), 20_000_000_000_000),
            "1.5x slower than real time"
        );
        assert_eq!(
            format_pace(Duration::from_secs(30), 15_000_000_000_000),
            "2x slower than real time"
        );
        assert_eq!(
            format_pace(Duration::from_secs(30), 3_010_000_000_000),
            "10x slower than real time"
        );
        assert_eq!(
            format_pace(Duration::from_secs(30), 60_000_000_000_000),
            "2x faster than real time"
        );
    }

    #[test]
    fn shell_condition_quotes_only_when_needed() {
        assert_eq!(shell_condition("@led1 lit"), "@led1 lit");
        assert_eq!(shell_condition("no diagnostics"), "no diagnostics");
        assert_eq!(shell_condition("@led1.i > 5mA"), "'@led1.i > 5mA'");
        assert_eq!(
            shell_condition("serial contains \"ready\""),
            "'serial contains \"ready\"'"
        );
    }

    #[test]
    fn test_timeout_hint_names_step_timeout_flag() {
        let mut reply = Reply {
            session: "tests/slow.test".into(),
            t_ps: 25_000_000_000,
            state: State::Paused,
            data: Data::None,
            error: Some(Error::new(Code::Cancelled, "cancelled", "")),
            previous_crash: None,
        };
        let step = Step::For(SimDuration::from_ms(3_000));
        say_step_timeout(
            &mut reply,
            &step,
            0,
            3_000_000_000_000,
            STEP_BUDGET,
            STEP_BUDGET,
            StepTimeoutHint::Test,
        );
        let error = reply.error.unwrap();
        assert_eq!(error.code, Code::StepTimeout);
        assert_eq!(
            error.message,
            "step stopped at t=25ms (asked for 3s) after the 30s wall-time limit (1200x slower than real time)"
        );
        assert!(error.hint.contains("--step-timeout"), "{}", error.hint);
    }
}
