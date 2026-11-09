//! What a session says to the viewers that watch it, and how it reads what they send. The CLI's
//! WebSocket and a browser's Worker both speak through this, so a viewer gets the same messages
//! from either. `packages/viewer/src/protocol.ts` is generated from these types, and
//! `scripts/bless.sh` rewrites it.

use std::path::Path;
use std::time::Duration;

use orbe_scene::View;
use serde::Serialize;
use serde_json::{Value, json};

use crate::Files;
use crate::command::{Command, spec};
use crate::live::{Channels, FrameState, Live, Pace, Sample};
use crate::reply::{Code, Data, Envelope, Error, Reply};

/// `play` advances this much simulated time per tick at most, one tick of wall time at speed 1,
/// so a slow engine plays in slow motion and a command waits at most one tick. A `step` sends
/// viewers a frame at most this often.
pub const TICK: Duration = Duration::from_millis(16);

/// The commands a viewer sends.
pub(crate) const SENT: [&str; 4] = ["press", "release", "play", "pause"];

/// A message's `type`: a type with the one value `$name`.
macro_rules! tag {
    ($tag:ident, $name:literal) => {
        #[derive(Serialize)]
        #[cfg_attr(test, derive(schemars::JsonSchema), schemars(inline))]
        pub(crate) enum $tag {
            #[serde(rename = $name)]
            Tag,
        }
    };
}

tag!(SceneTag, "scene");
tag!(FrameTag, "frame");
tag!(ReplyTag, "reply");
tag!(ClosedTag, "closed");

/// The scene a session opened, sent once per `open`, with the refs each frame array follows.
#[derive(Serialize)]
#[cfg_attr(
    test,
    derive(schemars::JsonSchema),
    schemars(extend("allOf" = [{ "$ref": "./view#/$defs/View" }]))
)]
pub(crate) struct SceneMessage<'a> {
    r#type: SceneTag,
    session: &'a str,
    channels: &'a Channels,
    #[serde(flatten)]
    #[cfg_attr(test, schemars(skip))]
    view: &'a View,
}

/// The `scene` message of `view` in the session `session`.
pub(crate) fn scene_message(session: &str, channels: &Channels, view: &View) -> Value {
    let message = SceneMessage {
        r#type: SceneTag::Tag,
        session,
        channels,
        view,
    };
    serde_json::to_value(message).expect("a scene message serializes")
}

/// The circuit at `t_ps`, indexed by the scene's `channels`.
#[derive(Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub(crate) struct FrameMessage<'a> {
    r#type: FrameTag,
    seq: u64,
    t_ps: u64,
    state: FrameState,
    /// 0 for low, 1 for high, per `channels.pins`.
    #[cfg_attr(test, schemars(extend("items" = { "enum": [0, 1] })))]
    pins: &'a [u8],
    /// Mean current in amperes since the previous frame, per `channels.leds`. The frame after a
    /// page joins carries the present current.
    led_a: Vec<f64>,
    /// 1 for closed, 0 for open, per `channels.switches`.
    #[cfg_attr(test, schemars(extend("items" = { "enum": [0, 1] })))]
    switches: &'a [u8],
    /// On a frame from a `play` tick: the speed `play` runs at, always 1 for now.
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f64>,
    /// On a frame from a `play` tick: simulated time over wall time in that tick, as the session
    /// achieved it. Under 1 when the engine runs slower than real time.
    #[serde(skip_serializing_if = "Option::is_none")]
    ratio: Option<f64>,
    /// How many diagnostics the session has, warnings and the fault.
    diagnostics: usize,
    /// The diagnostic that faulted the session, as `orbe snapshot` prints it, such as
    /// `error[solver.convergence] Newton did not converge ...`. Absent unless `state` is
    /// `faulted`.
    #[serde(skip_serializing_if = "Option::is_none")]
    fault: Option<&'a str>,
}

/// The reply to the viewer's command `id`. Only the viewer that sent the command gets it. `id`
/// is null when the command was not JSON with a number `id`.
#[derive(Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub(crate) struct ReplyMessage<'a> {
    r#type: ReplyTag,
    id: Option<u64>,
    reply: Envelope<'a>,
}

/// The session ended, with why, such as `` `orbe close` ended the session``. Nothing follows.
#[derive(Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub(crate) struct ClosedMessage<'a> {
    r#type: ClosedTag,
    reason: &'a str,
}

/// A viewer's command, `{"id": 1, "cmd": "press", "ref": "@btn1"}`, parsed by the CLI's own
/// table, as `orbe press @btn1` would be, with the `id` its reply carries back.
pub fn command(text: &str) -> (Option<u64>, Result<Command, Error>) {
    let usage = |message: String| {
        Error::new(
            Code::Usage,
            message,
            "a viewer sends press, release, play or pause, as {\"id\": 1, \"cmd\": \"press\", \"ref\": \"@btn1\"}",
        )
    };
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return (None, Err(usage("a viewer's command is not JSON".into())));
    };
    let id = message["id"].as_u64();
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
    let message = ReplyMessage {
        r#type: ReplyTag::Tag,
        id,
        reply: reply.envelope(),
    };
    let message = serde_json::to_value(message).expect("a reply message serializes");
    (reply, message)
}

/// The last message a viewer gets, with why the session ended.
pub fn closed(reason: &str) -> Value {
    let message = ClosedMessage {
        r#type: ClosedTag::Tag,
        reason,
    };
    serde_json::to_value(message).expect("a closed message serializes")
}

/// The `scene` message of the catalog part `id` drawn on its own, as a part's page shows it,
/// with no session behind it: the part is the one instance, named `id` with each `-` written
/// `_`, and no channel follows it. `parts` holds each catalog part's files by `<id>/<path>`, the part's and every part it is
/// built from. A part that is missing or does not parse is the error, as text.
pub fn part_scene(id: &str, parts: Vec<(String, Vec<u8>)>) -> Result<Value, String> {
    let block = json!({ "format": 1, "block": id, "version": "0.0.0", "title": id, "board": id, "license": "MIT" });
    let scene = json!({ "format": 1, "parts": { id.replace('-', "_"): { "type": id } } });
    let files = Files::memory([
        (orbe_scene::BLOCK, block.to_string().into_bytes()),
        ("scene.jsonc", scene.to_string().into_bytes()),
    ])
    .with_catalog(parts);
    let view = orbe_scene::view(&files, Path::new("scene.jsonc")).map_err(|found| {
        found
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    Ok(scene_message(id, &Channels::default(), &view))
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
        let frame = FrameMessage {
            r#type: FrameTag::Tag,
            seq: self.seq,
            t_ps: sample.t_ps,
            state: sample.state,
            pins: &sample.pins,
            led_a,
            switches: &sample.switches,
            speed: pace.map(|pace| pace.speed),
            ratio: pace.map(|pace| pace.ratio),
            diagnostics: sample.diagnostics,
            fault: sample.fault.as_deref(),
        };
        let frame = serde_json::to_value(frame).expect("a frame serializes");
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
        assert_eq!(id, Some(7));
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
        assert_eq!(none, None);
        assert_eq!(
            garbled.unwrap_err().message,
            "a viewer's command is not JSON"
        );
    }
}
