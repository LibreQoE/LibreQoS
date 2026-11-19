//! The RouterOS API protocol half: word framing (the length prefixes) and
//! the `=key=value` word and sentence conventions.
//!
//! Pure and synchronous: words in, words out. No socket and no timeout
//! lives here; the TCP connection, login, and framed sentence exchange
//! live in [`crate::mikrotik_transport`]. The typed rows the identity model
//! consumes are assembled by the adapter engine from the document's field
//! maps, so no per-command sentence mapper lives here.

use std::fmt;

/// Most words allowed in one reply sentence, so a peer streaming endless
/// words cannot grow a read unboundedly.
pub(crate) const MAX_SENTENCE_WORDS: usize = 256;

/// Appends `word` to `out`, length-prefixed in the 1-byte form (below
/// 0x80) or the 2-byte form (up to 0x3FFF). Longer words are an error:
/// the login and print exchanges never need them.
pub(crate) fn encode_word(out: &mut Vec<u8>, word: &str) -> Result<(), &'static str> {
    let bytes = word.as_bytes();
    let length = bytes.len();
    if length < 0x80 {
        out.push(length as u8); // guarded by the branch above
    } else if length < 0x4000 {
        out.push(0x80 | (length >> 8) as u8); // length < 0x4000 keeps this below 0xC0
        out.push((length & 0xFF) as u8); // masked to one byte
    } else {
        return Err("word exceeds the two-byte length form");
    }
    out.extend_from_slice(bytes);
    Ok(())
}

/// Decodes a word length from its 1-byte form (`first` below 0x80) or
/// 2-byte form (`first` in 0x80..0xC0 with `second` present). The 3-byte
/// and longer forms (minimum word 0x4000 bytes) are rejected: the login
/// and print exchanges never carry words that large.
pub(crate) fn word_length(first: u8, second: Option<u8>) -> Result<usize, &'static str> {
    match first {
        0x00..=0x7F => Ok(usize::from(first)),
        0x80..=0xBF => {
            let Some(second) = second else {
                return Err("truncated two-byte length prefix");
            };
            Ok((usize::from(first & 0x3F) << 8) | usize::from(second))
        }
        _ => Err("word length exceeds the two-byte length form"),
    }
}

/// The value of the `=key=value` word in `sentence`, if present.
pub(crate) fn word_value<'a>(sentence: &'a [String], key: &str) -> Option<&'a str> {
    let prefix = format!("={key}=");
    sentence
        .iter()
        .find(|word| word.starts_with(&prefix))
        .map(|word| &word[prefix.len()..])
}

/// The `=message=` value of a `!trap`/`!fatal` sentence, or a fixed
/// placeholder when the router sent none.
pub(crate) fn message_of(sentence: &[String]) -> &str {
    word_value(sentence, "message").unwrap_or("no message given")
}

/// A neighbour row learned from the RouterOS `/ip/neighbor` table.
///
/// A field the router omitted or sent empty stays `None`: the router's
/// neighbour rows are commonly sparse (for example passive listeners with
/// no address and no identity).
#[derive(Clone, Debug)]
pub struct ApiNeighbour {
    /// IP address the neighbour advertises, when present.
    pub address: Option<String>,
    /// MAC address of the neighbour, when present.
    pub mac_address: Option<String>,
    /// RouterOS identity the neighbour reports, when present.
    pub identity: Option<String>,
    /// Local interface the neighbour was seen on, when present.
    pub interface: Option<String>,
}

impl fmt::Display for ApiNeighbour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {} {}",
            self.address.as_deref().unwrap_or("-"),
            self.mac_address.as_deref().unwrap_or("-"),
            self.identity.as_deref().unwrap_or("-"),
            self.interface.as_deref().unwrap_or("-"),
        )
    }
}

/// One `/interface/print` row: the fields the details model tracks, with
/// everything the router omitted or sent empty kept absent.
#[derive(Clone, Debug)]
pub struct RouterOsInterface {
    /// The row id (`=.id=`), the device's stable handle for the interface.
    pub id: String,
    /// The interface name, when present.
    pub name: Option<String>,
    /// The raw RouterOS type string (`ether`, `vlan`, `bridge`, …).
    pub raw_type: Option<String>,
    /// The interface's own MAC, when present.
    pub mac_address: Option<String>,
    /// The `running` flag: `Some(true)` is running.
    pub running: Option<bool>,
    /// The `disabled` flag: `Some(true)` is disabled.
    pub disabled: Option<bool>,
}

/// One `/ip/address/print` row.
#[derive(Clone, Debug)]
pub struct RouterOsAddress {
    /// The address with prefix length (`192.168.1.1/24`), when present.
    pub address: Option<String>,
    /// The network the address sits in, when present.
    pub network: Option<String>,
    /// The interface name the address binds to, when present.
    pub interface: Option<String>,
}

/// One `/interface/bridge/host/print` row: one MAC the bridge has
/// learned, with the bridge and port it was seen on. A row without a MAC
/// is no evidence; the other fields stay absent when the router omitted
/// them.
#[derive(Clone, Debug)]
pub struct RouterOsBridgeHost {
    /// The bridge the host was learned on.
    pub bridge: Option<String>,
    /// The interface the host was seen on (the port).
    pub on_interface: Option<String>,
    /// The learned MAC.
    pub mac_address: Option<String>,
    /// How long since the entry was last seen, when the router reports it.
    pub age: Option<String>,
    /// The `dynamic` flag: learned by the dataplane, not configured.
    pub dynamic: Option<bool>,
    /// The `local` flag: one of the bridge's own MACs.
    pub local: Option<bool>,
    /// The `external` flag: learned from an external FDB source.
    pub external: Option<bool>,
}

/// One `/ip/arp/print` row: an IP-to-MAC binding the router's ARP table
/// carries. An entry the router is still resolving (`complete=false`)
/// carries no MAC; the other fields stay absent when the router omitted
/// them.
#[derive(Clone, Debug)]
pub struct RouterOsArp {
    /// The bound IP address (without prefix length).
    pub address: Option<String>,
    /// The learned MAC, when the entry is complete.
    pub mac_address: Option<String>,
    /// The interface the binding sits on.
    pub interface: Option<String>,
    /// The `dynamic` flag: learned by the dataplane, not configured.
    pub dynamic: Option<bool>,
    /// The `complete` flag: the router has resolved and still holds the
    /// MAC.
    pub complete: Option<bool>,
    /// The `disabled` flag: a statically configured, disabled binding.
    pub disabled: Option<bool>,
}

/// One `/interface/bridge/port/print` row: one port's membership in a
/// software bridge. `status` is the router's own text (newer RouterOS
/// versions report it; 6.49 does not) and is kept raw — the model never
/// derives activity from it.
#[derive(Clone, Debug)]
pub struct RouterOsBridgePort {
    /// The bridge the port belongs to.
    pub bridge: Option<String>,
    /// The member interface's name.
    pub interface: Option<String>,
    /// The port's VLAN id (the untagged VLAN), when reported.
    pub pvid: Option<u32>,
    /// The `frame-types` setting, as the router spells it.
    pub frame_types: Option<String>,
    /// The router's raw port status, when it reports one.
    pub status: Option<String>,
    /// The `inactive` flag: the port is not currently forwarding.
    pub inactive: Option<bool>,
    /// The `disabled` flag: administratively disabled.
    pub disabled: Option<bool>,
    /// The `dynamic` flag: created by the dataplane, not configured.
    pub dynamic: Option<bool>,
}

/// One `/ip/firewall/nat/print` row: one firewall NAT rule as the router
/// spells it. Every field is raw configuration text — address lists,
/// ranges, and ports are kept verbatim and parsed nowhere in the model —
/// and stays absent when the router omitted it. A row the router sent
/// with no mapped field at all is no rule; the engine drops it.
#[derive(Clone, Debug)]
pub struct RouterOsNatRule {
    /// The `/ip/firewall/nat` chain the rule belongs to.
    pub chain: Option<String>,
    /// The rule's action (`masquerade`, `src-nat`, `dst-nat`, …).
    pub action: Option<String>,
    /// The interface traffic must arrive on.
    pub in_interface: Option<String>,
    /// The interface traffic must leave through.
    pub out_interface: Option<String>,
    /// The source address, prefix, list, or range as configured.
    pub src_address: Option<String>,
    /// The destination address, prefix, list, or range as configured.
    pub dst_address: Option<String>,
    /// The translated address or range as configured.
    pub to_addresses: Option<String>,
    /// The `disabled` flag: the rule is configured but not active.
    pub disabled: Option<bool>,
    /// The rule's comment, verbatim.
    pub comment: Option<String>,
}

/// One `/interface/bridge/vlan/print` row: one bridge's VLAN
/// configuration. The router sends the interface fields as
/// comma-separated lists; they are split and kept in router order, with
/// the `current-*` columns — what the bridge is actually using — stored
/// beside the configured ones.
#[derive(Clone, Debug)]
pub struct RouterOsBridgeVlan {
    /// The bridge the row configures.
    pub bridge: Option<String>,
    /// The configured VLAN ids, in router order.
    pub vlan_ids: Vec<u32>,
    /// The configured tagged interfaces.
    pub tagged: Vec<String>,
    /// The configured untagged interfaces.
    pub untagged: Vec<String>,
    /// The tagged interfaces currently in use.
    pub current_tagged: Vec<String>,
    /// The untagged interfaces currently in use.
    pub current_untagged: Vec<String>,
}

#[cfg(test)]
/// Encodes one word through [`encode_word`] and returns the full bytes.
pub(crate) fn encoded(word: &str) -> Vec<u8> {
    let mut out = Vec::new();
    encode_word(&mut out, word).expect("word should encode");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_length_round_trips_both_forms() {
        for length in [0usize, 1, 0x7F, 0x80, 0xFF, 0x100, 0x3FFF] {
            let word = "a".repeat(length);
            let bytes = encoded(&word);
            let decoded =
                word_length(bytes[0], bytes.get(1).copied()).expect("encoded prefix should decode");
            assert_eq!(decoded, length);
            assert_eq!(&bytes[bytes.len() - length..], word.as_bytes());
        }
    }

    #[test]
    fn words_past_the_two_byte_form_are_rejected_on_encode() {
        let mut out = Vec::new();
        let word = "a".repeat(0x4000);
        assert_eq!(
            encode_word(&mut out, &word).unwrap_err(),
            "word exceeds the two-byte length form"
        );
        assert!(out.is_empty());
    }

    #[test]
    fn length_prefixes_of_longer_forms_are_rejected() {
        // 0xC0 and up begin the 3-, 4-, and 5-byte forms; the client
        // rejects them instead of reading continuation bytes.
        for first in [0xC0, 0xE0, 0xF0, 0xFF] {
            assert_eq!(
                word_length(first, None).unwrap_err(),
                "word length exceeds the two-byte length form"
            );
        }
    }

    #[test]
    fn a_truncated_two_byte_prefix_is_rejected() {
        assert_eq!(
            word_length(0x80, None).unwrap_err(),
            "truncated two-byte length prefix"
        );
    }

    #[test]
    fn reply_words_are_matched_by_key() {
        let sentence = vec![
            "!re".to_owned(),
            "=name=FSW-Office-Core".to_owned(),
            "=disabled=false".to_owned(),
        ];
        assert_eq!(word_value(&sentence, "name"), Some("FSW-Office-Core"));
        assert_eq!(word_value(&sentence, "disabled"), Some("false"));
        assert_eq!(word_value(&sentence, "absent"), None);
        // The value may itself contain `=`: only the first `=key=` runs
        // is the separator.
        let tricky = vec!["=name=a=b=c".to_owned()];
        assert_eq!(word_value(&tricky, "name"), Some("a=b=c"));
    }

    #[test]
    fn trap_messages_fall_back_when_absent() {
        let with_message = vec!["!trap".to_owned(), "=message=bad user name".to_owned()];
        assert_eq!(message_of(&with_message), "bad user name");
        let bare = vec!["!trap".to_owned()];
        assert_eq!(message_of(&bare), "no message given");
    }
}
