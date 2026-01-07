# LQOS Bus

The bus acts as an intermediary through which `lqosd` and various clients communicate. The Python integration system, `lqtop` and other CLI tools use it to interact with `lqosd`.

The bus is intentionally not stable between releases. So if you integrate with it, you will need to update your integration *every time* you update `lqosd`. This:

* Allows for rapid iteration on the back-end.
* Avoids the need for version negotiation (CBOR tolerates unknown enum variants).
* Uses CBOR serialization with length-prefixed chunked framing for large payloads.

> If you'd like a stable API, please consider the HTTP API provided with Insight.

## Licensing

The LibreQoS Bus system is also separately licensed. We use the [AGPL-3.0](https://www.gnu.org/licenses/agpl-3.0.en.html) license for the bus system, with the explicit exception that LibreQoS itself and all products produced by LibreQoE LLC are exempt and licensed separately.

We made this decision to ensure that the bus remains free and open source, and clients that add functionality to LibreQoS will also remain free and open source---even if you host it as a service. However, we do not want to force LibreQoS itself to be AGPL-3.0, as we want to allow for commercial use of LibreQoS itself.

Any contributions to the bus system must include permission to use this license, *including* "carve-outs" for LibreQoS itself and all products produced by LibreQoE LLC.

## Contributing

Please read `CONTRIBUTING.md` before submitting patches.

- SPDX for contributions: `AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception`.
- The exception terms are defined in `LICENSE` under AGPLv3 §7 “Additional permissions”.
