//! The netlist's `network`: one access point on the air and the subnet behind it, as
//! `orbe-net` runs them.

use serde::Deserialize;

use orbe_model::air::Channel;
use orbe_net::{Config, Mac, Security, Subnet};

/// The top-level `network`. A field out of its range is `netlist.syntax` at `network`.
#[derive(Deserialize)]
#[serde(try_from = "WireNetworkFields")]
pub(crate) struct WireNetwork(pub(crate) Config);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNetworkFields {
    ssid: String,
    bssid: String,
    channel: u8,
    #[serde(default)]
    passphrase: Option<String>,
    #[serde(default)]
    epoch_s: Option<u64>,
}

impl TryFrom<WireNetworkFields> for WireNetwork {
    type Error = String;

    fn try_from(wire: WireNetworkFields) -> Result<WireNetwork, String> {
        let bssid = mac(&wire.bssid)
            .filter(|mac| !mac.is_group())
            .ok_or("`bssid` is six hex octets joined by `:`, an individual address")?;
        let channel = Channel::new(wire.channel).ok_or("`channel` is 1 to 14")?;
        let security = match &wire.passphrase {
            None => Security::Open,
            Some(passphrase) => Security::Wpa2Psk { passphrase },
        };
        let config = Config::new(wire.ssid.as_bytes(), bssid, channel, security).ok_or(
            "`ssid` is 1 to 32 bytes, and `passphrase` 8 to 63 characters from space to `~`",
        )?;
        let mut subnet = Subnet::default();
        if let Some(epoch) = wire.epoch_s {
            subnet.epoch = epoch;
        }
        let config = config
            .with_subnet(subnet)
            .ok_or("`epoch_s` is at most 253383854055")?;
        Ok(WireNetwork(config))
    }
}

/// `aa:bb:cc:dd:ee:ff`, in either case.
fn mac(text: &str) -> Option<Mac> {
    let mut octets = [0; 6];
    let mut parts = text.split(':');
    for octet in &mut octets {
        let part = (parts.next())
            .filter(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_hexdigit()))?;
        *octet = u8::from_str_radix(part, 16).ok()?;
    }
    parts.next().is_none().then_some(Mac(octets))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_is_six_octets() {
        assert_eq!(
            mac("02:6F:72:62:65:01"),
            Some(Mac([2, 0x6f, 0x72, 0x62, 0x65, 1]))
        );
        for bad in [
            "02:6f:72:62:65",
            "02:6f:72:62:65:01:00",
            "2:6f:72:62:65:01",
            "zz:6f:72:62:65:01",
            "+2:6f:72:62:65:01",
        ] {
            assert_eq!(mac(bad), None, "{bad}");
        }
    }
}
