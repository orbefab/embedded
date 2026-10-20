//! `logic.vcd` read back, and the `observations.logic` summary it should agree with.

/// `{ref: {initial, edges: [{t_ps, level}]}}` read from a logic.vcd.
pub fn read_vcd(text: &str) -> serde_json::Value {
    let mut tokens = text.split_whitespace();
    let mut scope = "";
    let mut names = Vec::new();
    let mut signals = serde_json::Map::new();
    let mut at = 0;
    while let Some(token) = tokens.next() {
        match token {
            "$timescale" => assert_eq!(
                tokens.by_ref().take(3).collect::<Vec<_>>(),
                ["1", "ps", "$end"]
            ),
            "$scope" => scope = tokens.nth(1).unwrap(),
            "$var" => {
                let var: Vec<_> = tokens.by_ref().take(5).collect();
                names.push((var[2].to_string(), format!("#{scope}.logic.{}", var[3])));
            }
            "$upscope" | "$enddefinitions" | "$dumpvars" | "$end" => {}
            _ if token.starts_with('#') => at = token[1..].parse::<u64>().unwrap(),
            _ => {
                let (bit, id) = token.split_at(1);
                let level = if bit == "1" { "high" } else { "low" };
                let (_, key) = names.iter().find(|(name, _)| name == id).unwrap();
                match signals.get_mut(key) {
                    None => {
                        signals.insert(
                            key.clone(),
                            serde_json::json!({ "initial": level, "edges": [] }),
                        );
                    }
                    Some(signal) => signal["edges"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({ "t_ps": at, "level": level })),
                }
            }
        }
    }
    serde_json::Value::Object(signals)
}

/// `observations.logic` computed from `read_vcd`'s edges.
pub fn summary(signals: &serde_json::Value) -> serde_json::Value {
    let signals = signals.as_object().unwrap().iter().map(|(key, signal)| {
        let edges = signal["edges"].as_array().unwrap();
        let initial = &signal["initial"];
        let time = |edge: Option<&serde_json::Value>| edge.map(|edge| edge["t_ps"].clone());
        let summary = serde_json::json!({
            "initial": initial,
            "final": edges.last().map_or(initial, |edge| &edge["level"]),
            "count": edges.len(),
            "first_ps": time(edges.first()),
            "last_ps": time(edges.last()),
        });
        (key.clone(), summary)
    });
    serde_json::Value::Object(signals.collect())
}
