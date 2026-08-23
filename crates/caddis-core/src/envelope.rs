//! envelope.rs — strict envelope form + validation (CARD-0001 step 2; ENVELOPE-SCHEMA-v2).
//! Invalid -> Err(code, why). Valid -> Ok(Envelope). No allocation beyond input.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub v: u8,
    pub id: String,
    pub idem_key: String,
    pub r#type: String,
    pub from: String,
    pub to: String,
    pub body: String, // v0: opaque; schema'd bodies arrive with the real kernel (ENVELOPE-SCHEMA-v2)
    pub ts: String,
}

#[derive(Debug, PartialEq)]
pub struct EnvErr {
    pub code: &'static str,
    pub why: &'static str,
}

#[allow(clippy::too_many_arguments)] // the 8 args ARE the ENVELOPE-SCHEMA-v2 wire fields; a params struct would be a second, unvalidated Envelope type
pub fn validate(
    v: u8,
    id: &str,
    idem_key: &str,
    typ: &str,
    from: &str,
    to: &str,
    body: &str,
    ts: &str,
) -> Result<Envelope, EnvErr> {
    if v != 1 {
        return Err(EnvErr {
            code: "E-ENVELOPE",
            why: "bad version",
        });
    }
    if id.len() < 8 {
        return Err(EnvErr {
            code: "E-ENVELOPE",
            why: "bad id",
        });
    }
    if idem_key.is_empty() {
        return Err(EnvErr {
            code: "E-ENVELOPE",
            why: "bad idem_key",
        });
    }
    if typ.is_empty() || !typ.chars().next().unwrap().is_ascii_alphabetic() {
        return Err(EnvErr {
            code: "E-ENVELOPE",
            why: "bad type",
        });
    }
    if from.is_empty() || to.is_empty() {
        return Err(EnvErr {
            code: "E-ENVELOPE",
            why: "bad endpoints",
        });
    }
    if ts.is_empty() {
        return Err(EnvErr {
            code: "E-ENVELOPE",
            why: "bad ts",
        });
    }
    Ok(Envelope {
        v,
        id: id.into(),
        idem_key: idem_key.into(),
        r#type: typ.into(),
        from: from.into(),
        to: to.into(),
        body: body.into(),
        ts: ts.into(),
    })
}
