# Gate 3 external dependencies and release eligibility

Development completion is distinct from authorization or evidence of live payment.
Only Arcadestr is the implementation target. The abandoned wallet fork is retained
untouched and is not a dependency. No ADP Server or protocol changes are proposed here.

| Capability / owner | Current capability | Arcadestr mitigation | Residual risk | Improvement condition | Live eligibility impact |
|---|---|---|---|---|---|
| Hard routing-fee ceiling / wallet | No established standard NWC per-invoice ceiling in this adapter | Default provider-ceiling policy refuses dispatch without independent adapter assurance; optional reported-fee policy requires separate explicit risk consent, reserves remaining fees and halts on missing reports/overrun | In reported-fee mode a wallet can exceed desired limits before Arcadestr detects it; application limits do not constrain other credential holders | Documented, independently verified capability of an unmodified provider; never fabricate a NWC fee field | Preserved strict 250-sat fixture remains ineligible; weaker-mode development tests do not relax that fixture |
| Spending permission / wallet and operator | Fresh authenticated standard `get_info` on original dedicated connection advertises get_balance, get_info, list_transactions, lookup_invoice, get_budget; NOT pay_invoice; network regtest | Retain app #4 read-only; expose advertised methods without treating connection verification as consent | Capability advertisement is not payment permission or durability evidence | Separate explicit permission-change authorization followed by supported read-only verification | No live spending currently eligible |
| Outgoing payment lookup / wallet | Standard lookup_invoice supported; unknown/unsupported responses remain possible | Original hash, encrypted intent and irreversible admission; read-only reconciliation; never repay on uncertainty | Unavailable or ambiguous evidence may require manual investigation indefinitely | Stable provider lookup with validated original preimage and fee reporting | Unknown status blocks subsequent dispatch, not unrelated development |
| Atomic multi-recipient settlement / Lightning | Sequential payments are not atomic; no multi_pay_invoice dependency | Require original proof acknowledgment before next leg; accurate partial/uncertain UI; ownership requires validated aggregate receipt | First recipient may be paid while later payment/fulfillment fails | Separately designed protocol capability, if required; no atomicity claim for batching | Explicit partial-settlement risk; no live experiment authorized |
| Invoice preflight provenance / ADP protocol | Signed quote binds logical recipients/destinations/amounts; invoices disclosed sequentially, not all signing keys in the quote | Verify developer-signed listing and accepted terms, reconstruct exact split; persist original disclosure; validate BOLT11 signature/hash/amount/expiry and pin signing key before dispatch | Lightning node identity is not a Nostr recipient; original processor disclosure retains protocol trust assumptions | Agreed independently signed full preflight commitments, if stronger provenance is required | Cannot claim stronger provenance; not an excuse for wallet/server changes |
| Economic settlement / Lightning and ADP | Existing preimage proof and authorized aggregate-receipt model | Check original hash/preimage; recover proof without repayment; retain validated authorized receipt and source evidence | Preimage alone does not prove all economic assumptions under collusion | Separately specified stronger settlement evidence | Existing protocol trust model must be disclosed; no live durability claim |
| Dedicated-wallet paid durability / integration | Read-only persistence passed; paid restart/recovery has not been exercised | Extensive disposable DB/mock interruption/reopen/concurrency/recovery tests; live gate independently blocks quote, invoice and payment commands | Real provider outage, restart and fulfillment interoperability unverified | Separately authorized exact preserved fixture, supported permissions and fee assurance; independent paid evidence | Remains unverified and release-restricted, not automatically a development failure |

## Authorization boundary

`require_live_purchase_authorization()` fails closed independently of local buyer
approval. It protects quote creation (which preflights invoices), next-invoice
disclosure, legacy LNURL invoice creation and NWC dispatch. Changing local plan
limits, enabling optional reported-fee accounting, or reading old budget evidence
does not unlock it. Recovery checks existing payments and submits original proofs;
it never creates an admission or authorizes another payment.

No permission changes, new live quotes/invoices, payments, mining, reservation
replacement or commits are authorized by development completion.

## Gate 4 audit additions (Gate 3 remains complete)

| Capability / owner | Practical impact | Current mitigation / residual risk | Improvement condition | Development / live eligibility |
|---|---|---|---|---|
| Interoperable entitlement kind / protocol | NIP-103 remains an unallocated draft; the existing experimental grant profile cannot be advertised as a production standard | Retain existing explicit profile/discovery validation; no kind 1021 substitution; local grant and ownership tests do not establish interoperability | Agree an event-kind allocation and consuming-protocol profile across projects | Does not block payment orchestration development; interoperable grant release remains conditional |
| Aggregate split privacy / existing NIP-102 profile | Public aggregate receipts correlate buyer, listing, recipients, amounts and payment hashes | Approval UI discloses the correlation and distinguishes encrypted raw invoice/preimage/refund data; public correlation remains inherent to this profile | Agreed privacy-preserving interoperable receipt design if required | Current profile can be developed with informed disclosure; no new privacy guarantee or live execution claimed |
| Multiple saved NWC connections / optional Arcadestr UX | One connection per account with safe replacement/reconnection is already supported; simultaneous wallet picker and QR import are not required for purchase safety | Immutable approvals bind the original wallet/application identity; replacement invalidates review and cannot move unresolved payments to a new connection | Separate scoped UX feature reusing credential isolation and original-wallet recovery | Optional future improvement, not a Gate 4 blocker or permission to change credentials |

Arcadestr receipt/listing binding, false completion, stale-account callback and
Library synchronization defects found in Gate 4 are fixed in Arcadestr and are
**not** external dependencies. See `regtest-gate4-buy-flow-audit.md` for evidence.

## Receipt portability additions (Gate 3/4 remain complete)

Public receipt discovery is best effort, never guaranteed recovery of every
historical receipt. Relay retention/availability, capped timestamp pages and
omitted updates are disclosed; bounded overlap/backfill and manual signed-event
import provide complementary recovery paths. NIP-59 transport and multi-character
`#order` SDK filtering remain optional improvements; current discovery uses the
standard `#p` fallback without a parallel client. Missing verification evidence
stays INDETERMINATE rather than owned or automatically invalid.

Local imported ownership does not mint tokens or guarantee an ADP server can
retrieve the same receipt or serve the build. Existing independent server and
distribution authorization remain in effect. Details and practical mitigations:
`scripts/receipt-portability.md`.
