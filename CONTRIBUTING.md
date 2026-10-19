# Contributing

Bug reports, documentation fixes, and focused patches are welcome. Start with an issue before substantial feature work so we can agree on scope. x-talk aims to stay a small Windows dictation tool. macOS is on the roadmap but has no implementation or hardware acceptance commitment yet.

Use the setup and test commands in [README.md](README.md). Put scratch artifacts in `.tmp/`, which Git ignores. Keep changes focused, update affected documentation, and add regression coverage for substantive behavior changes. Run the relevant checks before sending a pull request; describe anything you could not verify. [SPEC.md](SPEC.md) records intended behavior; [AGENTS.md](AGENTS.md) contains development conventions.

Contributions are provided under Apache License 2.0. Submit only work you have the right to contribute. No copyright assignment or separate contributor agreement is required. Dependencies retain their own license obligations.

For bugs, include the app version/build identity from General, Windows version, steps to reproduce, expected/actual behavior, and relevant sanitized logs. Do not attach API keys, private transcripts, or recordings. Provider errors can depend on account permissions and quotas; identify the provider/model without credentials. Maintainers provide support as time permits.

For suspected vulnerabilities, follow [SECURITY.md](SECURITY.md) rather than opening a public issue with exploit details or secrets.

For native test isolation and Windows/WSL troubleshooting, use [the agent workflow](docs/AGENT_WORKFLOW.md). [The release guide](docs/RELEASING.md) explains versioning and distribution: normal pushes to main automatically publish after CI passes. Contributors can build and test without provider credentials or the private update-signing key.
