//! The elements the access point sends and reads, after IEEE 802.11-2020 9.4.2. Each element is
//! an ID octet, a length octet and that many octets (9.4.2.1, Table 9-92).

pub(crate) const SSID: u8 = 0;
pub(crate) const SUPPORTED_RATES: u8 = 1;
pub(crate) const DSSS: u8 = 3;
pub(crate) const TIM: u8 = 5;
pub(crate) const ERP: u8 = 42;
pub(crate) const RSN: u8 = 48;
pub(crate) const EXTENDED_RATES: u8 = 50;

/// The Supported Rates and BSS Membership Selectors element's rates (9.4.2.3), in 500 kb/s
/// with the top bit on a basic rate: 1, 2, 5.5 and 11 Mb/s basic, then 18, 24, 36 and 54 Mb/s.
/// The rates and their order are those of the access point in Wireshark's `wpa-Induction.pcap`.
pub(crate) const RATES: [u8; 8] = [0x82, 0x84, 0x8b, 0x96, 0x24, 0x30, 0x48, 0x6c];
/// The Extended Supported Rates and BSS Membership Selectors element's rates (9.4.2.12): 6, 9,
/// 12 and 48 Mb/s.
pub(crate) const EXTENDED: [u8; 4] = [0x0c, 0x12, 0x18, 0x60];

// RSNE cipher and AKM suites, OUI 00-0F-AC (9.4.2.24.2, 9.4.2.24.3).
const OUI: [u8; 3] = [0x00, 0x0f, 0xac];
pub(crate) const CCMP: [u8; 4] = [0x00, 0x0f, 0xac, 4];
const PSK: [u8; 4] = [0x00, 0x0f, 0xac, 2];
/// The AKM an RSNE without an AKM list means, IEEE 802.1X (9.4.2.24.1).
const IEEE8021X: [u8; 4] = [0x00, 0x0f, 0xac, 1];

/// The RSNE's body for WPA2-PSK (9.4.2.24): version 1, CCMP-128 group and pairwise, the PSK
/// AKM, and RSN Capabilities 0.
pub(crate) const RSN_PSK: [u8; 20] = [
    1, 0, // version
    OUI[0], OUI[1], OUI[2], 4, // group: CCMP-128
    1, 0, OUI[0], OUI[1], OUI[2], 4, // pairwise: CCMP-128
    1, 0, OUI[0], OUI[1], OUI[2], 2, // AKM: PSK
    0, 0, // RSN Capabilities
];

// Status codes (9.4.1.9, Table 9-50).
pub(crate) const INVALID_ELEMENT: u16 = 40;
const INVALID_GROUP_CIPHER: u16 = 41;
const INVALID_PAIRWISE_CIPHER: u16 = 42;
const INVALID_AKMP: u16 = 43;
const UNSUPPORTED_RSNE_VERSION: u16 = 44;
const ROBUST_MANAGEMENT_POLICY_VIOLATION: u16 = 31;

/// The MFPR bit of RSN Capabilities (9.4.2.24.4): the station requires PMF.
const MFPR: u8 = 1 << 6;

pub(crate) fn push(out: &mut Vec<u8>, id: u8, body: &[u8]) {
    out.push(id);
    out.push(body.len() as u8);
    out.extend(body);
}

/// The TIM element (9.4.2.5): DTIM Count, DTIM Period, Bitmap Control and the Partial
/// Virtual Bitmap of `aids`, the AIDs the access point holds frames for, in ascending order.
/// The bitmap runs from octet N1, the largest even number with every octet before it 0, to N2,
/// the last octet with a bit set. With no bit set it is one octet of 0 and N1 is 0. Bitmap
/// Control holds N1 / 2 in its upper seven bits. Its bit 0, for group frames held at a DTIM,
/// stays 0, since the access point holds none.
pub(crate) fn tim(
    out: &mut Vec<u8>,
    dtim_count: u8,
    dtim_period: u8,
    aids: impl Iterator<Item = u16>,
) {
    let mut bitmap = [0u8; 251];
    for aid in aids {
        bitmap[usize::from(aid / 8)] |= 1 << (aid % 8);
    }
    let (n1, n2) = match bitmap.iter().rposition(|&b| b != 0) {
        Some(n2) => (bitmap.iter().position(|&b| b != 0).unwrap_or(0) & !1, n2),
        None => (0, 0),
    };
    let mut body = vec![dtim_count, dtim_period, n1 as u8];
    body.extend(&bitmap[n1..=n2]);
    push(out, TIM, &body);
}

/// Each element of `body`, its ID and its contents, up to the first that runs past the end.
pub(crate) fn elements(mut body: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    std::iter::from_fn(move || {
        let (&id, rest) = body.split_first()?;
        let (&len, rest) = rest.split_first()?;
        let contents = rest.get(..usize::from(len))?;
        body = &rest[usize::from(len)..];
        Some((id, contents))
    })
}

/// The contents of the first element with `id` in `body`.
pub(crate) fn find(body: &[u8], id: u8) -> Option<&[u8]> {
    elements(body)
        .find(|&(found, _)| found == id)
        .map(|(_, contents)| contents)
}

/// Whether a station's RSNE asks for what the access point offers: its `group` cipher, CCMP-128
/// as a pairwise cipher, and the PSK AKM, without requiring PMF, which the access point does not
/// do (12.6.3). A field the RSNE leaves off takes its default, CCMP-128 for the ciphers and IEEE
/// 802.1X for the AKM (9.4.2.24.1). A refusal is its status code.
pub(crate) fn check_rsne(rsne: &[u8], group: &[u8]) -> Result<(), u16> {
    let mut at = 0;
    let mut take = |n: usize| -> Result<Option<&[u8]>, u16> {
        if at == rsne.len() {
            return Ok(None);
        }
        let field = rsne.get(at..at + n).ok_or(INVALID_ELEMENT)?;
        at += n;
        Ok(Some(field))
    };
    let version = take(2)?.ok_or(INVALID_ELEMENT)?;
    if version != [1, 0] {
        return Err(UNSUPPORTED_RSNE_VERSION);
    }
    if take(4)?.unwrap_or(&CCMP) != group {
        return Err(INVALID_GROUP_CIPHER);
    }
    let mut list = |default: [u8; 4], wanted: [u8; 4], refused: u16| -> Result<(), u16> {
        let Some(count) = take(2)? else {
            return if default == wanted {
                Ok(())
            } else {
                Err(refused)
            };
        };
        let count = u16::from_le_bytes([count[0], count[1]]);
        let mut found = false;
        for _ in 0..count {
            found |= take(4)?.ok_or(INVALID_ELEMENT)? == wanted;
        }
        if found { Ok(()) } else { Err(refused) }
    };
    list(CCMP, CCMP, INVALID_PAIRWISE_CIPHER)?;
    list(IEEE8021X, PSK, INVALID_AKMP)?;
    match take(2)? {
        Some(capabilities) if capabilities[0] & MFPR != 0 => {
            Err(ROBUST_MANAGEMENT_POLICY_VIOLATION)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tim_of(dtim_count: u8, dtim_period: u8, aids: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        tim(&mut out, dtim_count, dtim_period, aids.iter().copied());
        out
    }

    // Wireshark's wpa-Induction.pcap, frame 1: a beacon of SSID "Coherer" whose TIM is DTIM
    // Count 0, DTIM Period 1, Bitmap Control 0 and one bitmap octet of 0.
    #[test]
    fn a_tim_with_no_traffic_is_one_octet_of_zero() {
        assert_eq!(tim_of(0, 1, &[]), [0x05, 0x04, 0x00, 0x01, 0x00, 0x00]);
    }

    // 9.4.2.5: N1 is the largest even octet number with every bit before it 0, and Bitmap
    // Control holds N1 / 2 above its group bit.
    #[test]
    fn a_tim_bitmap_starts_at_the_even_octet_before_its_first_bit() {
        assert_eq!(tim_of(0, 1, &[1]), [5, 4, 0, 1, 0, 0x02]);
        assert_eq!(tim_of(0, 1, &[1, 9]), [5, 5, 0, 1, 0, 0x02, 0x02]);
        // AID 24 is octet 3, so N1 is 2 and the bitmap is octets 2 and 3.
        assert_eq!(tim_of(1, 2, &[24]), [5, 5, 1, 2, 0x02, 0x00, 0x01]);
        assert_eq!(tim_of(0, 1, &[2007]).len(), 2 + 3 + 1);
    }

    // Wireshark's wpa-Induction.pcap, frame 82: the station's RSNE asks for a TKIP group key.
    #[test]
    fn an_rsne_is_checked_against_ccmp_and_psk() {
        assert_eq!(check_rsne(&RSN_PSK, &CCMP), Ok(()));
        let tkip_group = [
            1, 0, 0, 0x0f, 0xac, 2, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 2, 0, 0,
        ];
        assert_eq!(check_rsne(&tkip_group, &CCMP), Err(INVALID_GROUP_CIPHER));
        let mut tkip_pairwise = RSN_PSK;
        tkip_pairwise[11] = 2;
        assert_eq!(
            check_rsne(&tkip_pairwise, &CCMP),
            Err(INVALID_PAIRWISE_CIPHER)
        );
        let mut eap = RSN_PSK;
        eap[17] = 1;
        assert_eq!(check_rsne(&eap, &CCMP), Err(INVALID_AKMP));
        assert_eq!(check_rsne(&[2, 0], &CCMP), Err(UNSUPPORTED_RSNE_VERSION));
        assert_eq!(check_rsne(&[], &CCMP), Err(INVALID_ELEMENT));
        // Version alone: CCMP-128 ciphers and the IEEE 802.1X AKM.
        assert_eq!(check_rsne(&[1, 0], &CCMP), Err(INVALID_AKMP));
        assert_eq!(check_rsne(&RSN_PSK[..16], &CCMP), Err(INVALID_ELEMENT));
        // Without RSN Capabilities.
        assert_eq!(check_rsne(&RSN_PSK[..18], &CCMP), Ok(()));
        // A TKIP group key, when the access point offers it.
        assert_eq!(check_rsne(&tkip_group, &tkip_group[2..6]), Ok(()));
        // PMF capable is fine; PMF required is refused, and so is a cut RSN Capabilities.
        let mut mfpc = RSN_PSK;
        mfpc[18] = 0x80;
        assert_eq!(check_rsne(&mfpc, &CCMP), Ok(()));
        let mut mfpr = RSN_PSK;
        mfpr[18] = 0xc0;
        assert_eq!(
            check_rsne(&mfpr, &CCMP),
            Err(ROBUST_MANAGEMENT_POLICY_VIOLATION)
        );
        assert_eq!(check_rsne(&RSN_PSK[..19], &CCMP), Err(INVALID_ELEMENT));
    }

    #[test]
    fn elements_stop_at_one_that_runs_past_the_end() {
        let body = [0, 2, b'h', b'i', 3, 1, 6, 5, 9, 1];
        let found: Vec<_> = elements(&body).collect();
        assert_eq!(found, [(0, &b"hi"[..]), (3, &[6][..])]);
        assert_eq!(find(&body, 3), Some(&[6][..]));
        assert_eq!(find(&body, 5), None);
    }
}
