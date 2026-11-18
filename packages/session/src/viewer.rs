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
use crate::live::{Channels, FrameState, Live, Pace, Sample, WorldQuantity};
use crate::reply::{Code, Data, Envelope, Error, Reply};

/// `play` advances this much simulated time per tick at most, one tick of wall time at speed 1,
/// so a slow engine plays in slow motion and a command waits at most one tick. A `step` sends
/// viewers a frame at most this often.
pub const TICK: Duration = Duration::from_millis(16);

/// The most UTF-8 bytes of serial text one frame carries. A page holds the same bound.
const SERIAL_FRAME: usize = 4096;

/// The commands a viewer sends.
pub(crate) const SENT: [&str; 5] = ["press", "release", "play", "pause", "set"];

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
    /// Each world quantity the scene declares, in table order, which each frame's `world`
    /// follows. Absent when the scene declares none.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    world: &'a [WorldQuantity],
    #[serde(flatten)]
    #[cfg_attr(test, schemars(skip))]
    view: &'a View,
}

/// The `scene` message of `view` in the session `session`.
pub(crate) fn scene_message(
    session: &str,
    channels: &Channels,
    world: &[WorldQuantity],
    view: &View,
) -> Value {
    let message = SceneMessage {
        r#type: SceneTag::Tag,
        session,
        channels,
        world,
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
    /// Each value per the scene's `world`: the value from `t_ps` on, or from where the
    /// quantity's last `set` lands when that is later, so a `set` shows while paused. Absent
    /// when the scene declares none.
    #[serde(skip_serializing_if = "Option::is_none")]
    world: Option<&'a [f64]>,
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
    /// UART text new since the previous frame. Absent when none arrived and the session's
    /// text is still what the previous frame ended on.
    #[serde(skip_serializing_if = "Option::is_none")]
    serial: Option<SerialText>,
}

/// UART text that reached the session since the previous frame. The session keeps one serial
/// text, the bytes of every USART in the order their stop bits ended, decoded as UTF-8 the way
/// `serial contains` searches it. It does not record which USART a byte came from, so this names
/// no port.
#[derive(Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub(crate) struct SerialText {
    /// UTF-8 byte index in the session's serial text where `text` starts.
    from: u64,
    /// The new text. With `replace` absent, a page that has shown `from` bytes appends it. With
    /// `replace` true, the session no longer has what the page showed past `from`, and this is
    /// the tail to show instead, at most 4096 bytes.
    text: String,
    /// Present and true when `text` replaces what a page shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    replace: Option<bool>,
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
            "a viewer sends press, release, play, pause or set, as {\"id\": 1, \"cmd\": \"press\", \"ref\": \"@btn1\"}",
        )
    };
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return (None, Err(usage("a viewer's command is not JSON".into())));
    };
    let id = message["id"].as_u64();
    let name = message["cmd"].as_str().unwrap_or_default();
    let parsed = match spec(name) {
        Some(spec) if SENT.contains(&name) => spec
            .command(&viewer_args(name, &message), Path::new(""))
            .map_err(|reason| Error::new(Code::Usage, reason, "")),
        _ => Err(usage(format!("`{name}` is not a command a viewer sends"))),
    };
    (id, parsed)
}

/// The words after the command name. `set` takes `quantity` and `value`. The others take `ref`
/// and, where the table allows it, `speed`.
fn viewer_args(name: &str, message: &Value) -> Vec<String> {
    if name == "set" {
        let quantity = message["quantity"].as_str().unwrap_or("").to_string();
        let value = match message.get("value") {
            Some(Value::Number(number)) => number.to_string(),
            Some(Value::String(text)) => text.clone(),
            _ => String::new(),
        };
        return vec![quantity, value];
    }
    let mut args = Vec::new();
    args.extend(message["ref"].as_str().map(String::from));
    args.extend(message.get("speed").map(Value::to_string));
    args
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
    Ok(scene_message(id, &Channels::default(), &[], &view))
}

/// The `scene` and `frame` messages a session's viewers get. It numbers the frames, and gives
/// each LED its mean current since the frame before. `shown` is the serial text the previous
/// frame ended on.
#[derive(Default)]
pub struct Feed {
    seq: u64,
    last: Option<Sample>,
    shown: String,
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
    /// it opened one, then a frame when the circuit shows something new or its serial text
    /// changed. `shown` stays as it was across `open`: the new text is not a continuation, so
    /// the frame carries `replace` and a page that missed `scene` still drops the old text.
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
        self.frame(live.sample()?, live.serial(), pace)
    }

    /// The `frame` message for `sample`, or `None` when it shows what the last one did and
    /// `serial` is still the text the previous frame ended on. An LED's current is its mean
    /// since the last frame, or its present current when no time passed. `serial` is the
    /// session's serial text, and the frame carries only the part a page has not been sent.
    pub fn frame(&mut self, sample: Sample, serial: &str, pace: Option<Pace>) -> Option<Value> {
        let update = serial_update(&mut self.shown, serial);
        if update.is_none() && self.last.as_ref() == Some(&sample) {
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
            world: (!sample.world.is_empty()).then_some(sample.world.as_slice()),
            speed: pace.map(|pace| pace.speed),
            ratio: pace.map(|pace| pace.ratio),
            diagnostics: sample.diagnostics,
            fault: sample.fault.as_deref(),
            serial: update,
        };
        let frame = serde_json::to_value(frame).expect("a frame serializes");
        self.seq += 1;
        self.last = Some(sample);
        Some(frame)
    }
}

/// The serial a frame should carry, and `shown` becomes the text that frame ends on. `None`
/// when `current` is still `shown`. A page that has shown `from` bytes appends `text`. Anything
/// else replaces what it shows, and a long text is cut to its last [`SERIAL_FRAME`] bytes.
fn serial_update(shown: &mut String, current: &str) -> Option<SerialText> {
    if let Some(extra) = current.strip_prefix(shown.as_str()) {
        if extra.is_empty() {
            return None;
        }
        if extra.len() <= SERIAL_FRAME {
            let from = shown.len() as u64;
            shown.push_str(extra);
            return Some(SerialText {
                from,
                text: extra.to_string(),
                replace: None,
            });
        }
    }
    let tail = serial_tail(current);
    let from = (current.len() - tail.len()) as u64;
    let text = tail.to_string();
    *shown = current.to_string();
    Some(SerialText {
        from,
        text,
        replace: Some(true),
    })
}

/// The end of `text`, at most [`SERIAL_FRAME`] bytes, on a character boundary.
fn serial_tail(text: &str) -> &str {
    if text.len() <= SERIAL_FRAME {
        return text;
    }
    let mut from = text.len() - SERIAL_FRAME;
    while !text.is_char_boundary(from) {
        from += 1;
    }
    &text[from..]
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

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
        let (_, set) = command(r#"{"id": 12, "cmd": "set", "quantity": "light", "value": 50}"#);
        assert_eq!(
            set.unwrap(),
            Command::parse("set light 50", Path::new(""))
                .unwrap()
                .unwrap()
        );
        let (_, dark) = command(r#"{"id": 13, "cmd": "set", "quantity": "light", "value": "0"}"#);
        assert_eq!(
            dark.unwrap(),
            Command::parse("set light 0", Path::new(""))
                .unwrap()
                .unwrap()
        );
        let (none, garbled) = command("press");
        assert_eq!(none, None);
        assert_eq!(
            garbled.unwrap_err().message,
            "a viewer's command is not JSON"
        );
    }

    fn paused() -> Sample {
        Sample {
            t_ps: 0,
            state: FrameState::Paused,
            pins: vec![],
            leds: vec![],
            switches: vec![],
            world: vec![],
            diagnostics: 0,
            fault: None,
        }
    }

    #[test]
    fn serial_text_arrives_once_and_a_shorter_log_replaces_it() {
        let mut feed = Feed::default();
        let sample = paused();
        let bare = feed.frame(sample.clone(), "", None).unwrap();
        assert!(bare.get("serial").is_none(), "{bare}");

        let first = feed.frame(sample.clone(), "ab", None).unwrap();
        assert_eq!(first["serial"], json!({ "from": 0, "text": "ab" }));
        assert!(feed.frame(sample.clone(), "ab", None).is_none());

        let next = feed.frame(sample.clone(), "abcd", None).unwrap();
        assert_eq!(next["serial"], json!({ "from": 2, "text": "cd" }));
        assert_eq!(next["seq"], first["seq"].as_u64().unwrap() + 1);

        let shrunk = feed.frame(sample.clone(), "ab", None).unwrap();
        assert_eq!(
            shrunk["serial"],
            json!({ "from": 0, "text": "ab", "replace": true })
        );

        let mut long = "é".repeat(3000);
        long.push('z');
        let bounded = feed.frame(sample.clone(), &long, None).unwrap();
        let text = bounded["serial"]["text"].as_str().unwrap();
        assert_eq!(bounded["serial"]["replace"], true);
        assert!(text.len() <= SERIAL_FRAME, "{}", text.len());
        assert!(long.ends_with(text), "{text}");
        let from = bounded["serial"]["from"].as_u64().unwrap() as usize;
        assert_eq!(from + text.len(), long.len());

        let more = feed
            .frame(sample.clone(), &format!("{long}!"), None)
            .unwrap();
        assert_eq!(
            more["serial"],
            json!({ "from": long.len() as u64, "text": "!" })
        );

        let cleared = feed.frame(sample, "", None).unwrap();
        assert_eq!(
            cleared["serial"],
            json!({ "from": 0, "text": "", "replace": true })
        );
    }
}
