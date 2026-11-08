//! A Block's live session for a browser Worker, through `wasm-bindgen`. It runs the scene reader
//! and the session `orbe` runs, and speaks to its viewer in the messages `orbe show`'s WebSocket
//! carries. It reads no clock: the Worker says how much wall time passed at each `play` tick.
//! `docs/wasm.md` is the guide.

use std::path::Path;
use std::time::Duration;

use orbe_session::viewer::{self, Feed, TICK};
use orbe_session::{Command, Files, Live};
use wasm_bindgen::prelude::*;

/// The files of one Block, gathered before it opens.
#[wasm_bindgen]
#[derive(Default)]
pub struct Block {
    own: Vec<(String, Vec<u8>)>,
    /// Each catalog part's files, by `<id>/<path>`.
    parts: Vec<(String, Vec<u8>)>,
}

#[wasm_bindgen]
impl Block {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Block {
        Block::default()
    }

    /// Adds one file: `parts/<id>/<path>` is a file of the catalog part `id`, and any other path
    /// is the Block's own, from its folder, such as `scene.jsonc` or `firmware/button.hex`.
    pub fn add(&mut self, path: &str, bytes: Vec<u8>) {
        match path.strip_prefix("parts/") {
            Some(part) => self.parts.push((part.to_string(), bytes)),
            None => self.own.push((path.to_string(), bytes)),
        }
    }
}

/// A Block's live session, and the messages it has for its viewer.
#[wasm_bindgen]
pub struct Session {
    live: Live,
    feed: Feed,
    out: Vec<String>,
}

impl Session {
    /// `open`, with the refusal as text.
    pub fn start(name: &str, block: Block) -> Result<Session, String> {
        let files = Files::memory(block.own).with_catalog(block.parts);
        let mut live = Live::new(name, files);
        let reply = live.run(&Command::Open(Path::new("scene.jsonc").to_path_buf()));
        if !reply.ok() {
            return Err(reply.text());
        }
        let mut session = Session {
            live,
            feed: Feed::default(),
            out: Vec::new(),
        };
        let joined = session.feed.join(&session.live);
        session.queue(joined);
        Ok(session)
    }

    fn queue(&mut self, messages: impl IntoIterator<Item = serde_json::Value>) {
        self.out
            .extend(messages.into_iter().map(|message| message.to_string()));
    }
}

#[wasm_bindgen]
impl Session {
    /// Opens the Block's `scene.jsonc` as `orbe open` does in its folder, in a session named
    /// `name`. Its `scene` message and first `frame` wait in `messages`. Throws an `Error` whose
    /// message is the refusal as `orbe open` prints it.
    pub fn open(name: &str, block: Block) -> Result<Session, JsError> {
        Session::start(name, block).map_err(|text| JsError::new(&text))
    }

    /// Runs one command as a viewer sends it, such as `{"id": 1, "cmd": "press", "ref":
    /// "@btn1"}`. Its `reply`, and a `frame` when the circuit shows something new, wait in
    /// `messages`.
    pub fn command(&mut self, text: &str) {
        let (reply, message) = viewer::answer(&mut self.live, text);
        self.queue([message]);
        let after = self.feed.after(&self.live, &reply);
        self.queue(after);
    }

    /// One `play` tick, `wall_ms` milliseconds of wall time after the last: the session advances
    /// by that much, up to one tick, and the tick's `frame` waits in `messages`. A paused session
    /// does not move.
    pub fn tick(&mut self, wall_ms: f64) {
        let wall = Duration::try_from_secs_f64(wall_ms / 1000.0).unwrap_or_default();
        let pace = self.live.tick(wall);
        let frame = self.feed.now(&self.live, pace);
        self.queue(frame);
    }

    /// Whether `play` runs, so the Worker keeps ticking.
    pub fn playing(&self) -> bool {
        self.live.playing()
    }

    /// The JSON text of each message since the last call, in order.
    pub fn messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.out)
    }
}

/// The JSON text of the `scene` message of the catalog part `id` drawn on its own, with no
/// session. `parts` holds the files of the part and of every part it is built from, each at
/// `parts/<id>/<path>`. It takes the `Block`, which is freed. Throws an `Error` that lists the
/// findings when the part is missing or does not parse.
#[wasm_bindgen(js_name = partScene)]
pub fn part_scene(id: &str, parts: Block) -> Result<String, JsError> {
    viewer::part_scene(id, parts.parts)
        .map(|message| message.to_string())
        .map_err(|text| JsError::new(&text))
}

/// The wall time between two `play` ticks, in milliseconds.
#[wasm_bindgen(js_name = tickMs)]
pub fn tick_ms() -> f64 {
    TICK.as_secs_f64() * 1000.0
}
