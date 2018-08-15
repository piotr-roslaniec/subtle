#![cfg_attr(feature = "nightly", feature(external_doc))]
#![cfg_attr(feature = "nightly", doc(include = "../README.md"))]
#![doc(html_logo_url = "https://doc.dalek.rs/assets/dalek-logo-clear.png")]
// put this after the #![doc(..)] so it appears as a footer:
//! Note that docs will only build on nightly Rust until
//! [RFC 1990 stabilizes](https://github.com/rust-lang/rust/issues/44732).

extern crate byteorder;
extern crate clear_on_drop;
extern crate core;
extern crate keccak;

#[cfg(test)]
extern crate strobe_rs;

mod strobe;

use strobe::Strobe128;

fn encode_usize(x: usize) -> [u8; 4] {
    use byteorder::{ByteOrder, LittleEndian};

    assert!(x < (u32::max_value() as usize));

    let mut buf = [0; 4];
    LittleEndian::write_u32(&mut buf, x as u32);
    buf
}

/// A transcript of a public-coin argument.
///
/// The prover's messages are added to the transcript using `commit`,
/// and the verifier's challenges can be computed using `challenge`.
///
/// # Usage
///
/// Implementations of proof protocols should take a `&mut Transcript`
/// as a parameter, **not** construct one internally.  This provides
/// three benefits:
///
/// 1.  It forces the API client to initialize their own transcript
/// using `Transcript::new()`.  Since that function takes a domain
/// separation string, this ensures that all proofs are
/// domain-separated.
///
/// 2.  It ensures that protocols are sequentially composable, by
/// running them on a common transcript.  (Since transcript instances
/// are domain-separated, it should not be possible to extract a
/// sub-protocol's challenges and commitments as a standalone proof).
///
/// 3.  It allows API clients to commit contextual data to the
/// proof statements prior to running the protocol, allowing them to
/// bind proof statements to arbitrary application data.
///
/// # Defining protocol behaviour with extension traits
///
/// This API is byte-oriented, while an actual protocol likely
/// requires typed data — for instance, a protocol probably wants to
/// receive challenge scalars, not challenge bytes.  The recommended
/// way to bridge this abstraction gap is to define a
/// protocol-specific extension trait.
///
/// For instance, consider a discrete-log based protocol which commits
/// to Ristretto points and requires challenge scalars for the
/// Ristretto group.  This protocol can define a protocol-specific
/// extension trait in its crate as follows:
/// ```
/// extern crate curve25519_dalek;
/// use curve25519_dalek::ristretto::CompressedRistretto;
/// use curve25519_dalek::scalar::Scalar;
///
/// extern crate merlin;
/// use merlin::Transcript;
///
/// trait TranscriptProtocol {
///     fn commit_point(&mut self, point: CompressedRistretto);
///     fn challenge_scalar(&mut self) -> Scalar;
/// }
///
/// impl TranscriptProtocol for Transcript {
///     fn commit_point(&mut self, point: CompressedRistretto) {
///         self.commit(b"pt", point.as_bytes());
///     }
///
///     fn challenge_scalar(&mut self) -> Scalar {
///         let mut buf = [0; 64];
///         self.challenge(b"sc", &mut buf);
///         Scalar::from_bytes_mod_order_wide(&buf)
///     }
/// }
/// # fn main() { }
/// ```
/// Now, the implementation of the protocol can call the
/// `challenge_scalar` method on any `Transcript` instance.
///
/// However, because the protocol-specific behaviour is defined in a
/// protocol-specific trait, different protocols can use the same
/// `Transcript` instance without imposing any extra type constraints.
#[derive(Clone)]
pub struct Transcript {
    strobe: Strobe128,
}

impl Transcript {
    /// Initialize a new transcript with the supplied `label`, which
    /// is used as a domain separator.
    ///
    /// # Note
    ///
    /// This function should be called by a protocol's API consumer,
    /// and *not* by the protocol implementation.
    pub fn new(label: &[u8]) -> Transcript {
        Transcript {
            strobe: Strobe128::new(label),
        }
    }

    /// Commit a prover's `message` to the transcript.
    ///
    /// The `label` parameter is metadata about the message, and is
    /// also committed to the transcript.
    pub fn commit(&mut self, label: &[u8], message: &[u8]) {
        let data_len = encode_usize(message.len());
        self.strobe.meta_ad(label, false);
        self.strobe.meta_ad(&data_len, true);
        self.strobe.ad(message, false);
    }

    /// Fill the supplied buffer with the verifier's challenge bytes.
    ///
    /// The `label` parameter is metadata about the challenge, and is
    /// also committed to the transcript.
    pub fn challenge(&mut self, label: &[u8], challenge_bytes: &mut [u8]) {
        let data_len = encode_usize(challenge_bytes.len());
        self.strobe.meta_ad(label, false);
        self.strobe.meta_ad(&data_len, true);
        self.strobe.prf(challenge_bytes, false);
    }
}

#[cfg(test)]
mod tests {
    use strobe_rs::OpFlags;
    use strobe_rs::SecParam;
    use strobe_rs::Strobe;

    use super::*;

    /// Test against a full strobe implementation to ensure we match the few
    /// operations we're interested in.
    struct TestTranscript {
        state: Strobe,
    }

    impl TestTranscript {
        /// Strobe init; meta-AD(label)
        pub fn new(label: &[u8]) -> TestTranscript {
            TestTranscript {
                state: Strobe::new(label.to_vec(), SecParam::B128),
            }
        }

        /// Strobe op: meta-AD(label || len(message)); AD(message)
        pub fn commit(&mut self, label: &[u8], message: &[u8]) {
            // metadata = label || len(message);
            let metaflags: OpFlags = OpFlags::A | OpFlags::M;
            let mut metadata: Vec<u8> = Vec::with_capacity(label.len() + 4);
            metadata.extend_from_slice(label);
            metadata.extend_from_slice(&encode_usize(message.len()));

            self.state
                .ad(message.to_vec(), Some((metaflags, metadata)), false);
        }

        /// Strobe op: meta-AD(label || len(challenge_bytes)); PRF into challenge_bytes
        pub fn challenge(&mut self, label: &[u8], challenge_bytes: &mut [u8]) {
            let prf_len = challenge_bytes.len();

            // metadata = label || len(challenge_bytes);
            let metaflags: OpFlags = OpFlags::A | OpFlags::M;
            let mut metadata: Vec<u8> = Vec::with_capacity(label.len() + 4);
            metadata.extend_from_slice(label);
            metadata.extend_from_slice(&encode_usize(prf_len));

            let bytes = self.state.prf(prf_len, Some((metaflags, metadata)), false);
            challenge_bytes.copy_from_slice(&bytes);
        }
    }

    /// Test a simple protocol with one message and one challenge
    #[test]
    fn equivalence_simple() {
        let mut real_transcript = Transcript::new(b"test protocol");
        let mut test_transcript = TestTranscript::new(b"test protocol");

        real_transcript.commit(b"some label", b"some data");
        test_transcript.commit(b"some label", b"some data");

        let mut real_challenge = [0u8; 32];
        let mut test_challenge = [0u8; 32];

        real_transcript.challenge(b"challenge", &mut real_challenge);
        test_transcript.challenge(b"challenge", &mut test_challenge);

        assert_eq!(real_challenge, test_challenge);
    }

    /// Test a complex protocol with multiple messages and challenges,
    /// with messages long enough to wrap around the sponge state, and
    /// with multiple rounds of messages and challenges.
    #[test]
    fn equivalence_complex() {
        let mut real_transcript = Transcript::new(b"test protocol");
        let mut test_transcript = TestTranscript::new(b"test protocol");

        let data = vec![99; 1024];

        real_transcript.commit(b"step1", b"some data");
        test_transcript.commit(b"step1", b"some data");

        let mut real_challenge = [0u8; 32];
        let mut test_challenge = [0u8; 32];

        for _ in 0..32 {
            real_transcript.challenge(b"challenge", &mut real_challenge);
            test_transcript.challenge(b"challenge", &mut test_challenge);

            assert_eq!(real_challenge, test_challenge);

            real_transcript.commit(b"bigdata", &data);
            test_transcript.commit(b"bigdata", &data);

            real_transcript.commit(b"challengedata", &real_challenge);
            test_transcript.commit(b"challengedata", &test_challenge);
        }
    }
}
