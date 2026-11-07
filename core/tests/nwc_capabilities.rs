#![cfg(feature = "native")]
use arcadestr_core::nwc_client::{parse_wallet_capabilities, NwcClientError};

#[test]
fn standard_get_info_reports_only_advertised_operations() {
    let info=parse_wallet_capabilities(r#"{"result_type":"get_info","result":{"methods":["get_info","get_balance","pay_invoice","lookup_invoice"],"network":"regtest","max_fee_msat":0}}"#).expect("standard envelope");
    assert!(info.supports("pay_invoice"));
    assert!(info.supports("lookup_invoice"));
    assert!(!info.supports("multi_pay_invoice"));
    assert_eq!(info.network.as_deref(), Some("regtest"));
    let read_only=parse_wallet_capabilities(r#"{"result_type":"get_info","result":{"methods":["get_info","lookup_invoice"],"network":"regtest"}}"#).expect("read only");
    assert!(!read_only.supports("pay_invoice"));
}
#[test]
fn unsupported_ambiguous_or_absent_capabilities_never_grant_support() {
    assert!(matches!(
        parse_wallet_capabilities(
            r#"{"result_type":"get_info","error":{"code":"NOT_IMPLEMENTED"}}"#
        ),
        Err(NwcClientError::VerificationUnsupported)
    ));
    for bad in [
        r#"{"result_type":"pay_invoice","result":{}}"#,
        r#"{"result_type":"get_info","result":{"methods":"pay_invoice"}}"#,
        r#"{"result_type":"get_info","result":{"methods":[1]}}"#,
    ] {
        assert!(parse_wallet_capabilities(bad).is_err());
    }
    let unknown = parse_wallet_capabilities(r#"{"result_type":"get_info","result":{}}"#)
        .expect("optional fields");
    assert!(!unknown.supports("pay_invoice"));
    assert_eq!(unknown.network, None);
}
