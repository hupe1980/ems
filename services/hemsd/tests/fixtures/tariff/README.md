# Seller quote fixtures

The fixed and dynamic compact JWS files contain the unmodified bytes of the two price sheets in `crates/ems-tariff/examples/`. Both were signed independently of the gateway with Node's Ed25519 implementation under the fixture key ID “EMS synthetic seller”. Only the public key is retained.

Each row of `quotes.json` gives a Berlin quarter start, fixed gross ct/kWh and dynamic gross ct/kWh. A null means the contract has not started. Both contracts start on 1 January 2027. The dynamic index stays at −2 net ct/kWh so the change at each window edge comes entirely from network charges.

| Terms | Independent decimal calculation | Gross ct/kWh |
|---|---|---|
| Fixed | (9.80 + 11.20 + 7.331) × 1.19 | 33.71389 |
| Dynamic, night | (−2 + 1.90 + 4 + 7.331) × 1.19 | 13.36489 |
| Dynamic, standard | (−2 + 1.90 + 10 + 7.331) × 1.19 | 20.50489 |
| Dynamic, peak | (−2 + 1.90 + 18 + 7.331) × 1.19 | 30.02489 |

The 7.331 ct/kWh subtotal sums the five net levies on the signed sheets. Annual standing charges are separate.
