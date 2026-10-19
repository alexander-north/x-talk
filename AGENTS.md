# Agent instructions

## Start here
- Read [the agent workflow](docs/AGENT_WORKFLOW.md) before implementation or native testing. It maps the code, test modes, Windows/WSL setup, and known build/test pitfalls.
- Read the relevant sections of `SPEC.md` for product behavior; use `README.md` for bootstrap commands and `docs/RELEASING.md` for distribution.
- Inspect `git status` before editing. Preserve existing work and limit staging to your own files. Scratch scripts from earlier sessions are not supported project tooling.
- Choose verification for the changed behavior. Isolated offline acceptance is the default native UI test; real microphone, live provider, and installer lifecycle tests have different requirements. Never substitute Cancel coverage for normal Stop coverage.

## Clarify before building
- Ask clarifying questions early and generously before substantial implementation. Establish the goal, users, scope, constraints, expected behavior, and acceptance criteria.
- Surface ambiguities and tradeoffs instead of silently choosing product behavior. Group related questions so they are easy to answer.
- Once the direction is clear, take ownership of implementation and verification without repeatedly asking for permission for routine work.

## Own verification
- Run as much testing as is feasible before handing work back for review. Set up and exercise the relevant local environment yourself.
- Use appropriate automated checks and, when applicable, integration tests, end-to-end tests, and a manual smoke test of the affected workflow.
- Fix failures caused by your changes. Add meaningful regression coverage for bugs and substantive behavior changes.
- Report what changed, what you verified, and any remaining gaps or blockers. State clearly when a check could not run.

## Keep the repository tidy
- Put temporary notes, working Markdown, plans, logs, screenshots, and other scratch artifacts in `.tmp/`, which is ignored by Git. Create it as needed.
- Keep durable source files and agreed documentation outside `.tmp/`. Never commit secrets or temporary artifacts.
- Keep changes focused and preserve unrelated work.

## Keep documentation lean and current
- Use `SPEC.md` as the development source of truth once the product discussion establishes it. Keep decisions, scope, acceptance criteria, and open questions there.
- Update affected documentation in the same change as the implementation. Remove obsolete guidance and avoid duplicate explanations or status documents.
- Keep `README.md` limited to orientation and the setup/run/test commands that actually exist.

## Look up library and API details
- Use Context7 for current library, framework, SDK, API, CLI, and cloud-service documentation, including syntax, configuration, migrations, setup, and library-specific debugging.
- Start with `resolve-library-id` unless an exact library ID was provided; choose the relevant official/version-specific match, then use `query-docs` with the concrete question.
- If Context7 cannot supply the answer, say so and fall back to official documentation or ask a focused question.
- General programming, business logic, refactoring, and code review do not require a documentation lookup by themselves.
