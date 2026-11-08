//! What a session says to the viewers that watch it, and how it reads what they send. The CLI's
//! WebSocket and a browser's Worker both speak through this, so a viewer gets the same messages
//! from either. `packages/viewer/src/protocol.ts` types them.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::command::{Command, spec};
use crate::live::{Live, Pace, Sample};
use crate::reply::{Code, Data, Error, Reply};

/// `play` advances this much simulated time per tick at most, one tick of wall time at speed 1,
/// so a slow engine plays in slow motion and a command waits at most one tick. A `step` sends
/// viewers a frame at most this often.
pub const TICK: Duration = Duration::from_millis(16);

/// The commands a viewer sends.
const SENT: [&str; 4] = ["press", "release", "play", "pause"];

/// A viewer's command, `{"id": 1, "cmd": "press", "ref": "@btn1"}`, parsed by the CLI's own
/// table, as `orbe press @btn1` would be, with the `id` its reply carries back.
pub fn command(text: &str) -> (Value, Result<Command, Error>) {
    let usage = |message: String| {
        Error::new(
            Code::Usage,
            message,
            "a viewer sends press, release, play or pause, as {\"id\": 1, \"cmd\": \"press\", \"ref\": \"@btn1\"}",
        )
    };
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return (
            Value::Null,
            Err(usage("a viewer's command is not JSON".into())),
        );
    };
    let id = message["id"].clone();
    let name = message["cmd"].as_str().unwrap_or_default();
    let parsed = match spec(name) {
        Some(spec) if SENT.contains(&name) => {
            let mut args: Vec<String> = Vec::new();
            args.extend(message["ref"].as_str().map(String::from));
            args.extend(message.get("speed").map(Value::to_string));
            spec.command(&args, Path::new(""))
                .map_err(|reason| Error::new(Code::Usage, reason, ""))
        }
        _ => Err(usage(format!("`{name}` is not a command a viewer sends"))),
    };
    (id, parsed)
}

/// Runs a viewer's command text on `live`: the reply, and the message that carries it back to
/// that viewer alone.
pub fn answer(live: &mut Live, text: &str) -> (Reply, Value) {
    let (id, command) = command(text);
    let reply = match command {
        Ok(command) => live.run(&command),
        Err(error) => live.refuse(error),
    };
    let message = json!({ "type": "reply", "id": id, "reply": reply.json() });
    (reply, message)
}

/// The last message a viewer gets, with why the session ended.
pub fn closed(reason: &str) -> Value {
    json!({ "type": "closed", "reason": reason })
}

/// The `scene` and `frame` messages a session's viewers get. It numbers the frames, and gives
/// each LED its mean current since the frame before.
#[derive(Default)]
pub struct Feed {
    seq: u64,
    last: Option<Sample>,
}

impl Feed {
    /// What a viewer that joins gets: the open scene, then a frame of each LED's present
    /// current, so a new viewer never sees a mean over time no viewer watched.
    pub fn join(&mut self, live: &Live) -> Vec<Value> {
        self.last = None;
        live.scene()
            .into_iter()
            .chain(self.now(live, None))
            .collect()
    }

    /// What every viewer gets after a command ran, a viewer's or a client's: the new scene when
    /// it opened one, then a frame when the circuit shows something new.
    pub fn after(&mut self, live: &Live, reply: &Reply) -> Vec<Value> {
        let mut out = Vec::new();
        if let Data::Opened { .. } = reply.data {
            self.last = None;
            out.extend(live.scene());
        }
        out.extend(self.now(live, None));
        out
    }

    /// A frame of `live` at `now`, when it shows something new. `pace` is a `play` tick's.
    pub fn now(&mut self, live: &Live, pace: Option<Pace>) -> Option<Value> {
        self.frame(live.sample()?, pace)
    }

    /// The `frame` message for `sample`, or `None` when it shows what the last one did. An
    /// LED's current is its mean since the last frame, or its present current when no time
    /// passed.
    pub fn frame(&mut self, sample: Sample, pace: Option<Pace>) -> Option<Value> {
        if self.last.as_ref() == Some(&sample) {
            return None;
        }
        let led_a: Vec<f64> = match &self.last {
            Some(last) if sample.t_ps > last.t_ps && last.leds.len() == sample.leds.len() => {
                let span = (sample.t_ps - last.t_ps) as f64;
                sample
                    .leds
                    .iter()
                    .zip(&last.leds)
                    .map(|((_, now), (_, before))| (now - before) / span)
                    .collect()
            }
            _ => sample.leds.iter().map(|(i, _)| *i).collect(),
        };
        let mut frame = json!({
            "type": "frame",
            "seq": self.seq,
            "t_ps": sample.t_ps,
            "state": sample.state.as_str(),
            "pins": sample.pins,
            "led_a": led_a,
            "switches": sample.switches,
            "diagnostics": sample.diagnostics,
        });
        if let Some(pace) = pace {
            frame["speed"] = json!(pace.speed);
            frame["ratio"] = json!(pace.ratio);
        }
        self.seq += 1;
        self.last = Some(sample);
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_viewer_s_command_parses_as_the_cli_s() {
        let (id, press) = command(r#"{"id": 7, "cmd": "press", "ref": "@btn1"}"#);
        assert_eq!(id, 7);
        assert_eq!(
            press.unwrap(),
            Command::parse("press @btn1", Path::new(""))
                .unwrap()
                .unwrap()
        );
        let (_, play) = command(r#"{"id": 8, "cmd": "play"}"#);
        assert_eq!(play.unwrap(), Command::Play);
        let (_, step) = command(r#"{"id": 9, "cmd": "step", "ref": "1ms"}"#);
        assert_eq!(
            step.unwrap_err().message,
            "`step` is not a command a viewer sends"
        );
        let (_, unnamed) = command(r#"{"id": 10, "cmd": "press", "ref": "btn1"}"#);
        assert_eq!(unnamed.unwrap_err().code, Code::Usage);
        let (_, fast) = command(r#"{"id": 11, "cmd": "play", "speed": 2}"#);
        assert_eq!(fast.unwrap_err().message, "takes no arguments; usage: play");
        let (none, garbled) = command("press");
        assert_eq!(none, Value::Null);
        assert_eq!(
            garbled.unwrap_err().message,
            "a viewer's command is not JSON"
        );
    }
}
