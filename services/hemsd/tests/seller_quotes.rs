//! Independently signed seller sheets and gross import prices at Berlin boundaries.
use std::process::Command;

#[test]
fn signed_cli_quotes_match_the_seller_examples() {
    let rows: Vec<(&str, Option<&str>, Option<&str>)> =
        serde_json::from_str(include_str!("fixtures/tariff/quotes.json")).unwrap();
    for (instant, fixed, dynamic) in rows {
        for (kind, expected) in [("fixed", fixed), ("dynamic", dynamic)] {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_hemsd"));
            cmd.current_dir(env!("CARGO_MANIFEST_DIR")).args([
                "price-sheet",
                &format!("tests/fixtures/tariff/{kind}.jws"),
                "--signed",
                "--at",
                instant,
            ]);
            if kind == "dynamic" {
                cmd.arg("--index-ct=-2");
            }
            let output = cmd.output().unwrap();
            assert_eq!(
                output.status.success(),
                expected.is_some(),
                "{instant} {kind}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            if let Some(price) = expected {
                let quote: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(quote["gross_import_ct_per_kwh"], price);
                assert_eq!(quote["signer"]["kid"], "EMS synthetic seller");
                assert_eq!(
                    quote["signer"]["fingerprint"],
                    "tybE09fzokcY0Hs--Ok0Foqa6zQSvRX_Cub6l7rtXzo"
                );
            } else {
                assert!(String::from_utf8_lossy(&output.stderr).contains("validity"));
            }
        }
    }
}
