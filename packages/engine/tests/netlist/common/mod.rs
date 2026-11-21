use orbe_engine::netlist;

pub fn with_parts(analysis: &str, parts: &str) -> String {
    format!(
        r#"{{
      "version": 1,
      "seed": 7,
      "analysis": {analysis},
      "ground": "gnd",
      "nets": ["gnd", "a"],
      "parts": [{parts}]
    }}"#
    )
}

pub fn errors(json: &str) -> Vec<String> {
    let err = netlist::parse(json.as_bytes()).unwrap_err();
    err.iter()
        .map(|d| serde_json::to_value(d).unwrap())
        .map(|d| format!("{} {} {}", d["code"], d["refs"], d["data"]))
        .collect()
}
