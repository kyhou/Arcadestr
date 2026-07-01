---
source: Context7 API + docs.rs official Rustdoc
library: lightning-invoice
package: lightning-invoice
version: 0.32.0
topic: BOLT11 test invoice construction with known preimage/payment hash
fetched: 2026-06-24T00:00:00Z
official_docs: https://docs.rs/lightning-invoice/0.32.0/lightning_invoice/
---

# lightning-invoice 0.32: parseable BOLT11 invoice for tests

Official Rustdoc says to parse with `str::parse::<Bolt11Invoice>()`, construct with `InvoiceBuilder`, serialize with `Display` / `ToString`, and set `Currency`, `payment_hash`, `payment_secret`, timestamp, and CLTV before `build_signed`.

## 0.32-compatible helper with known preimage

```rust
use bitcoin::hashes::{sha256, Hash as _};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use lightning_invoice::{Bolt11Invoice, Currency, InvoiceBuilder};
use lightning_types::payment::PaymentSecret;
use std::time::Duration;

fn test_bolt11_invoice_with_known_preimage() -> ([u8; 32], Bolt11Invoice) {
    let preimage = [7u8; 32];
    let payment_hash = sha256::Hash::hash(&preimage);
    let payment_secret = PaymentSecret([42u8; 32]);

    let private_key = SecretKey::from_slice(&[
        0xe1, 0x26, 0xf6, 0x8f, 0x7e, 0xaf, 0xcc, 0x8b,
        0x74, 0xf5, 0x4d, 0x26, 0x9f, 0xe2, 0x06, 0xbe,
        0x71, 0x50, 0x00, 0xf9, 0x4d, 0xac, 0x06, 0x7d,
        0x1c, 0x04, 0xa8, 0xca, 0x3b, 0x2d, 0xb7, 0x34,
    ]).expect("valid test secret key");

    let invoice = InvoiceBuilder::new(Currency::Regtest)
        .amount_milli_satoshis(123_000)
        .description("test invoice".to_owned())
        .payment_hash(payment_hash)
        .payment_secret(payment_secret)
        .duration_since_epoch(Duration::from_secs(1_700_000_000))
        .min_final_cltv_expiry_delta(144)
        .build_signed(|hash| Secp256k1::new().sign_ecdsa_recoverable(hash, &private_key))
        .expect("test invoice builds and signs");

    let encoded = invoice.to_string();
    let reparsed = encoded
        .parse::<Bolt11Invoice>()
        .expect("serialized invoice parses as BOLT11");

    assert_eq!(reparsed.payment_hash(), &payment_hash);
    assert_eq!(sha256::Hash::hash(&preimage), *reparsed.payment_hash());
    assert_eq!(reparsed.amount_milli_satoshis(), Some(123_000));
    assert_eq!(reparsed.currency(), Currency::Regtest);

    (preimage, reparsed)
}
```

## Documented builder shape

Rustdoc's `InvoiceBuilder` example uses:

```rust
let invoice = InvoiceBuilder::new(Currency::Bitcoin)
    .description("Coins pls!".into())
    .payment_hash(payment_hash)
    .payment_secret(PaymentSecret([42u8; 32]))
    .current_timestamp()
    .min_final_cltv_expiry_delta(144)
    .build_signed(|hash| Secp256k1::new().sign_ecdsa_recoverable(hash, &private_key))
    .unwrap();

assert!(invoice.to_string().starts_with("lnbc1"));
```

## Relevant APIs

- `InvoiceBuilder::new(Currency::...)`
- `.amount_milli_satoshis(u64)`
- `.description(String)` or `.description_hash(sha256::Hash)`
- `.payment_hash(sha256::Hash)`
- `.payment_secret(lightning_types::payment::PaymentSecret)`
- `.current_timestamp()` or `.duration_since_epoch(Duration)`
- `.min_final_cltv_expiry_delta(u64)`
- `.build_signed(|&Message| RecoverableSignature)`
- `Bolt11Invoice::payment_hash() -> &sha256::Hash`
- `Bolt11Invoice::amount_milli_satoshis() -> Option<u64>`
- `str::parse::<Bolt11Invoice>()`
