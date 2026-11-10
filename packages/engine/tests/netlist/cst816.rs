//! A `cst816` touch schedule: each point is after t = 0, at least 2000 ps after the previous
//! one, and each coordinate is from 0 to 4095.

use orbe_engine::diagnostic::{Diagnostic, ScheduleRule, ValueRule};
use orbe_engine::netlist::{self, Chip, Model};

fn chip(touches: &str) -> String {
    format!(
        r#"{{
      "version": 0,
      "seed": 0,
      "analysis": {{ "transient": {{ "stop_ps": 1000000000, "max_step_ps": 1000000, "grid_ps": 1000000 }} }},
      "ground": "gnd",
      "nets": ["gnd", "vcc", "scl", "sda", "int", "rst"],
      "parts": [{{
        "name": "u2",
        "kind": "cst816",
        "pins": {{ "VCC": "vcc", "GND": "gnd", "SCL": "scl", "SDA": "sda", "INT": "int", "RST": "rst" }},
        "low_drop": {{ "value": 0.0, "unit": "V", "source": "user" }},
        "low_resistance": {{ "value": 133.3, "unit": "ohm", "source": "user" }},
        "input_threshold": {{
          "low": {{ "value": 0.3, "unit": "1", "source": "user" }},
          "high": {{ "value": 0.7, "unit": "1", "source": "user" }}
        }},
        "touches": {touches}
      }}]
    }}"#
    )
}

fn rejected(touches: &str) -> Vec<Diagnostic> {
    netlist::parse(chip(touches).as_bytes()).unwrap_err()
}

#[test]
fn a_point_after_t_0_is_kept() {
    let netlist =
        netlist::parse(chip(r#"[{ "t_ps": 2000, "x": 291, "y": 69, "down": true }]"#).as_bytes())
            .unwrap();
    let Model::Chip(Chip::Cst816(dev)) = &netlist.parts().next().unwrap().1.model else {
        panic!("u2 is a cst816");
    };
    assert_eq!(dev.touches.len(), 1);
    assert_eq!(dev.touches[0].at.as_ps(), 2000);
    assert_eq!(dev.touches[0].to.x, 291);
    assert_eq!(dev.touches[0].to.y, 69);
    assert!(dev.touches[0].to.down);
}

#[test]
fn a_touch_at_t_0_is_at_zero() {
    let err = rejected(r#"[{ "t_ps": 0, "x": 1, "y": 2, "down": true }]"#);
    assert!(err.iter().any(|d| matches!(
        d,
        Diagnostic::NetlistSchedule {
            field,
            event: 0,
            rule: ScheduleRule::AtZero,
            ..
        } if *field == "touches"
    )));
}

#[test]
fn a_touch_within_2000_ps_is_too_close() {
    let err = rejected(
        r#"[{ "t_ps": 2000, "x": 1, "y": 2, "down": true }, { "t_ps": 3000, "x": 3, "y": 4, "down": false }]"#,
    );
    assert!(err.iter().any(|d| matches!(
        d,
        Diagnostic::NetlistSchedule {
            field,
            event: 1,
            rule: ScheduleRule::TooClose,
            ..
        } if *field == "touches"
    )));
}

#[test]
fn a_coordinate_past_4095_is_span() {
    let err = rejected(r#"[{ "t_ps": 2000, "x": 4096, "y": 0, "down": true }]"#);
    assert!(err.iter().any(|d| matches!(
        d,
        Diagnostic::NetlistValue {
            field,
            rule: ValueRule::Span,
            ..
        } if field == "touches[0].x"
    )));
}
