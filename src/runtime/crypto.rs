//! The `[Crypto]` namespace: one-shot digests, HMAC, and CSPRNG bytes.
//!
//! `[Crypto]Digest` — sha256/sha512/sha1/md5/blake3 over a String's UTF-8 bytes
//! or a Bytes value, answering the raw digest as `Bytes` (compose with `toHex`
//! or `Base64.encode:` for text forms). MD5 is not cryptography anymore — it
//! lives here to keep the hashes together.
//!
//! `[Crypto]Hmac` — keyed MACs over the SHA family, plus `verifySha256:…`,
//! which compares in constant time (the naive `computed == received` is a
//! timing side channel — examples should never teach it).
//!
//! `[Crypto]Random` — bytes straight from the operating system's CSPRNG. The
//! seedable `Random` class is for simulations; this one is for secrets.

use crate::error::QuoinError;
use crate::value::{NativeClassBuilder, ObjectPayload, Value};

use hmac::digest::core_api::BlockSizeUser;
use hmac::{Mac, SimpleHmac};
use sha2::Digest;

/// The octets a digest input denotes: a String's UTF-8 bytes or a Bytes
/// value's contents. Anything else is a TypeError naming the selector.
fn octets(selector: &str, args: &[Value<'_>], idx: usize) -> Result<Vec<u8>, QuoinError> {
    if let Some(Value::Object(o)) = args.get(idx) {
        match &o.borrow().payload {
            ObjectPayload::String(s) => return Ok(s.as_bytes().to_vec()),
            ObjectPayload::Bytes(b) => return Ok(b.to_vec()),
            _ => {}
        }
    }
    Err(QuoinError::TypeError {
        expected: "String or Bytes".to_string(),
        got: args
            .get(idx)
            .map_or("None".to_string(), |v| v.type_name().to_string()),
        msg: format!("{selector} takes a String (hashed as UTF-8) or Bytes"),
    })
}

fn one_shot<D: Digest>(data: &[u8]) -> Vec<u8> {
    D::digest(data).to_vec()
}

fn hmac_bytes<D>(selector: &str, args: &[Value<'_>]) -> Result<Vec<u8>, QuoinError>
where
    D: Digest + BlockSizeUser,
{
    let message = octets(selector, args, 0)?;
    let key = octets(selector, args, 1)?;
    let mut mac = <SimpleHmac<D>>::new_from_slice(&key).expect("HMAC accepts any key length");
    mac.update(&message);
    Ok(mac.finalize().into_bytes().to_vec())
}

pub fn build_crypto_digest_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[Crypto]Digest", Some("Object"))
        .abstract_class()
        .class_doc(
            "One-shot cryptographic digests (and MD5, kept with the hashes despite being \
             broken for security use). Each method hashes a String's UTF-8 bytes or a \
             Bytes value and answers the raw digest as Bytes — `toHex` for the usual text \
             form, `Base64.encode:` for wire formats.\n\n\
             ```\n\
             ([Crypto]Digest.sha256:'abc').toHex.starts?:'ba7816bf'     \"* -> true\n\
             ```",
        )
        .sdk_class_method("sha256:", |host, _r, args| {
            Ok(host.new_bytes(one_shot::<sha2::Sha256>(&octets("sha256:", &args, 0)?)))
        })
        .returns("Bytes")
        .doc(
            "The SHA-256 digest (32 bytes).\n\n\
             ```\n\
             ([Crypto]Digest.sha256:'abc').count     \"* -> 32\n\
             ```",
        )
        .sdk_class_method("sha512:", |host, _r, args| {
            Ok(host.new_bytes(one_shot::<sha2::Sha512>(&octets("sha512:", &args, 0)?)))
        })
        .returns("Bytes")
        .doc(
            "The SHA-512 digest (64 bytes).\n\n\
             ```\n\
             ([Crypto]Digest.sha512:'abc').count     \"* -> 64\n\
             ```",
        )
        .sdk_class_method("sha1:", |host, _r, args| {
            Ok(host.new_bytes(one_shot::<sha1::Sha1>(&octets("sha1:", &args, 0)?)))
        })
        .returns("Bytes")
        .doc(
            "The SHA-1 digest (20 bytes). Broken for collision resistance — for legacy \
             interop (git object ids, old protocols), not for new designs.\n\n\
             ```\n\
             ([Crypto]Digest.sha1:'abc').count     \"* -> 20\n\
             ```",
        )
        .sdk_class_method("md5:", |host, _r, args| {
            Ok(host.new_bytes(one_shot::<md5::Md5>(&octets("md5:", &args, 0)?)))
        })
        .returns("Bytes")
        .doc(
            "The MD5 digest (16 bytes). Not cryptography anymore — checksums and legacy \
             interop only.\n\n\
             ```\n\
             ([Crypto]Digest.md5:'abc').count     \"* -> 16\n\
             ```",
        )
        .sdk_class_method("blake3:", |host, _r, args| {
            let data = octets("blake3:", &args, 0)?;
            Ok(host.new_bytes(blake3::hash(&data).as_bytes().to_vec()))
        })
        .returns("Bytes")
        .doc(
            "The BLAKE3 digest (32 bytes) — the modern fast one.\n\n\
             ```\n\
             ([Crypto]Digest.blake3:'abc').count     \"* -> 32\n\
             ```",
        )
}

pub fn build_crypto_hmac_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[Crypto]Hmac", Some("Object"))
        .abstract_class()
        .class_doc(
            "Keyed message authentication (HMAC) over the SHA family. Message and key are \
             each a String (UTF-8) or Bytes; the MAC comes back as Bytes. To CHECK a \
             received MAC use `verifySha256:message:key:` — it compares in constant time, \
             where `==` on the recomputed Bytes would leak timing.\n\n\
             ```\n\
             ([Crypto]Hmac.sha256:'msg' key:'secret').count     \"* -> 32\n\
             ```",
        )
        .sdk_class_method("sha256:key:", |host, _r, args| {
            Ok(host.new_bytes(hmac_bytes::<sha2::Sha256>("sha256:key:", &args)?))
        })
        .returns("Bytes")
        .doc(
            "The HMAC-SHA-256 of a message under a key (32 bytes).\n\n\
             ```\n\
             ([Crypto]Hmac.sha256:'msg' key:'secret').count     \"* -> 32\n\
             ```",
        )
        .sdk_class_method("sha512:key:", |host, _r, args| {
            Ok(host.new_bytes(hmac_bytes::<sha2::Sha512>("sha512:key:", &args)?))
        })
        .returns("Bytes")
        .doc(
            "The HMAC-SHA-512 of a message under a key (64 bytes).\n\n\
             ```\n\
             ([Crypto]Hmac.sha512:'msg' key:'secret').count     \"* -> 64\n\
             ```",
        )
        .sdk_class_method("sha1:key:", |host, _r, args| {
            Ok(host.new_bytes(hmac_bytes::<sha1::Sha1>("sha1:key:", &args)?))
        })
        .returns("Bytes")
        .doc(
            "The HMAC-SHA-1 of a message under a key (20 bytes) — legacy interop (TOTP \
             and friends), not new designs.\n\n\
             ```\n\
             ([Crypto]Hmac.sha1:'msg' key:'secret').count     \"* -> 20\n\
             ```",
        )
        .sdk_class_method("verifySha256:message:key:", |host, _r, args| {
            let expected = octets("verifySha256:message:key:", &args, 0)?;
            let message = octets("verifySha256:message:key:", &args, 1)?;
            let key = octets("verifySha256:message:key:", &args, 2)?;
            let mut mac = <SimpleHmac<sha2::Sha256>>::new_from_slice(&key)
                .expect("HMAC accepts any key length");
            mac.update(&message);
            Ok(host.new_bool(mac.verify_slice(&expected).is_ok()))
        })
        .returns("Boolean")
        .doc(
            "Whether a received MAC is the HMAC-SHA-256 of the message under the key, \
             compared in CONSTANT TIME — use this to check MACs, never `==` on the \
             recomputed Bytes (equality bails at the first differing byte, leaking how \
             much of a guess was right).\n\n\
             ```\n\
             var mac = [Crypto]Hmac.sha256:'msg' key:'k'\n\
             [Crypto]Hmac.verifySha256:mac message:'msg' key:'k'     \"* -> true\n\
             ```",
        )
}

pub fn build_crypto_random_class() -> NativeClassBuilder {
    NativeClassBuilder::new("[Crypto]Random", Some("Object"))
        .abstract_class()
        .class_doc(
            "Bytes from the operating system's CSPRNG — for keys, tokens, and salts. The \
             seedable `Random` class is for simulations and tests; this one is for \
             secrets (and is deliberately not seedable).\n\n\
             ```\n\
             ([Crypto]Random.bytes:16).count     \"* -> 16\n\
             ```",
        )
        .sdk_class_method("bytes:", |host, _r, args| {
            let n = crate::arg!(args, Int, 0);
            let n = usize::try_from(n).map_err(|_| {
                QuoinError::ValueError(format!("[Crypto]Random bytes: needs a count >= 0, got {n}"))
            })?;
            let mut buf = vec![0u8; n];
            getrandom::fill(&mut buf)
                .map_err(|e| QuoinError::Other(format!("OS CSPRNG failed: {e}")))?;
            Ok(host.new_bytes(buf))
        })
        .returns("Bytes")
        .doc(
            "N bytes from the OS CSPRNG.\n\n\
             ```\n\
             ([Crypto]Random.bytes:32).count     \"* -> 32\n\
             ```",
        )
}
