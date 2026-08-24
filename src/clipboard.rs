//! Copying text to the system clipboard via OSC 52.
//!
//! OSC 52 is a terminal escape sequence that asks the *terminal* to set the
//! clipboard. That choice is deliberate: it needs no dependency, and because
//! the request travels over the same channel as the rest of the output, it
//! works when GitLoom is run over SSH, where a library talking to the local
//! X11/Wayland/Win32 clipboard would set the clipboard of the wrong machine.
//!
//! The tradeoff is that it is advisory. Windows Terminal, WezTerm, kitty,
//! iTerm2, Alacritty and recent tmux honor it; some terminals ignore it and a
//! few require it to be enabled. Nothing is reported back, so [`copy`]
//! returning `Ok` means the request was written, not that the clipboard
//! changed. Callers should word their status message accordingly.

use std::io::{self, Write};

/// Ask the terminal to put `text` on the system clipboard.
///
/// Writes straight to stdout, which is correct even under ratatui's alternate
/// screen: this is a control sequence rather than drawing, and the next frame
/// repaints over nothing.
pub fn copy(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout();
    // `52` is the clipboard operation, `c` selects the system clipboard
    // (as opposed to `p`, the X11 primary selection), and BEL terminates.
    write!(stdout, "\x1b]52;c;{}\x07", base64(text.as_bytes()))?;
    stdout.flush()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding, as OSC 52 requires.
///
/// Hand-written to keep the dependency list at what the app actually needs: the
/// only thing ever encoded here is a 40-character hex OID.
fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        // Pack the chunk into the low 24 bits, then read it back out six bits
        // at a time; missing input bytes stay zero and become `=` below.
        let packed = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | (b as u32) << (16 - 8 * i));

        for slot in 0..4 {
            if slot <= chunk.len() {
                let index = (packed >> (18 - 6 * slot)) & 0b11_1111;
                out.push(ALPHABET[index as usize] as char);
            } else {
                out.push('=');
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RFC 4648 test vectors, which pin all three padding cases.
    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_covers_the_high_end_of_the_alphabet() {
        // 0xFB 0xFF encodes to the last alphabet entries, catching a table or
        // shift that is wrong only at the top of the range.
        assert_eq!(base64(&[0xFB, 0xFF]), "+/8=");
        assert_eq!(base64(&[0xFF, 0xFF, 0xFF]), "////");
    }

    #[test]
    fn encoded_length_is_always_a_multiple_of_four() {
        for len in 0..64 {
            let bytes = vec![b'a'; len];
            assert_eq!(base64(&bytes).len() % 4, 0, "length {len}");
        }
    }

    /// The only payload this module encodes in practice.
    #[test]
    fn a_full_oid_round_trips_to_a_known_string() {
        let oid = "3c4e0537c4a1f0a1b2c3d4e5f60718293a4b5c6d";
        let encoded = base64(oid.as_bytes());
        assert_eq!(encoded.len(), 56, "40 bytes -> 56 base64 characters");
        assert!(encoded.starts_with("M2M0ZTA1Mzdj"));
    }
}
