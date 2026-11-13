# Contributing to zcrowd

Thanks for taking the time to contribute.

zcrowd is a Rust command line tool, a small library (`zcrowd-core`) and a one-file web page. They tell you how much your
Zcash amount looks like other amounts on the public chain. The project holds itself to the standards of Zcash development,
and this guide follows the structure and the style rules of the
[librustzcash CONTRIBUTING.md](https://github.com/zcash/librustzcash/blob/main/CONTRIBUTING.md).

zcrowd is built with AI coding agents (Claude). Contributions are reviewed the same way, whatever tool wrote them.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [I Have a Question](#i-have-a-question)
- [I Want To Contribute](#i-want-to-contribute)
- [Reporting Bugs](#reporting-bugs)
- [Reporting Security Issues](#reporting-security-issues)
- [Suggesting Enhancements](#suggesting-enhancements)
- [Styleguides](#styleguides)
- [Tests](#tests)
- [Continuous Integration](#continuous-integration)
- [Licence](#licence)

## Code of Conduct

This project and everyone in it follow the
[Zcash Code of Conduct](https://github.com/zcash/zcash/blob/master/code_of_conduct.md). Be respectful. Disagree about the
code, not about the person. Report unacceptable behavior as that document describes.

## I Have a Question

Read the [README](README.md) and [docs/PRIVACY.md](docs/PRIVACY.md) first. They explain what each command does and what
leaves your machine.

Then search the [issues](https://github.com/Kylne700/zcrowd/issues). If an issue fits, add your question there. If none
fits, open a new issue and give:

- what you ran and what you expected,
- the output you got,
- the output of `zcrowd --version` and `rustc -V`, and your operating system.

If you found the answer yourself and it was not in the docs, open a pull request that adds it.

## I Want To Contribute

> ### Legal Notice
> When you contribute, you must agree that you wrote 100% of the content, that you have the rights to it, and that it may
> be provided under the project licence (MIT, see [Licence](#licence)).

### Project Structure

- `crates/zcrowd-core`: the library. It decodes transactions, ingests blocks, reads and writes the data files and scores a
  crowd. Blocks are read through the `BlockSource` trait. The lightwalletd client and the fixtures both implement it.
- `crates/zcrowd`: the command line binary (`check`, `report`, `verify`, `sync`) and the network sandbox.
- `web/`: the one-file web page. `web/index.html` is built by `node web/build.mjs` from `web/index.template.html`. Do not
  edit `web/index.html` by hand.
- `fixtures/`: the committed sample and block fixtures. Tests run offline on them.
- `tests/`: `e2e.sh` and `audit.sh`.
- `docs/PRIVACY.md`: the privacy claims and the check behind each one.

### Rust Version

The workspace is Rust edition 2024 with `rust-version = "1.90"` in `Cargo.toml`. That is the minimum supported Rust
version. Do not raise it without asking first. If you touch dependencies, check that it still holds:

```
cargo +1.90.0 check --workspace --all-targets --locked
```

CI builds and tests with Rust 1.97.1. Use that version for the commands below.

### Dependencies

`Cargo.lock` is committed and CI uses `--locked`. A new dependency needs a reason in the pull request. It must pass
`cargo deny --locked check` with the policy in `deny.toml` (advisories, licences, bans, sources). Do not add an `ignore`
entry to `deny.toml` without a written reason, as the existing entries have.

## Reporting Bugs

### Before Submitting a Bug Report

- Check that it is a bug and not a mistake on your side. For example, `check` refuses data written by another zcrowd
  version on purpose: run `zcrowd sync` again.
- Search the [issues](https://github.com/Kylne700/zcrowd/issues) for the same problem.
- Collect: operating system and CPU, `rustc -V`, `zcrowd --version`, the exact command, its full output, and whether you
  can reproduce it every time.
- Use the bundled sample (README section 2) for a reproduction if you can. Do not paste your real addresses, seed phrases
  or amounts that you want to keep private.

### How Do I Submit a Good Bug Report?

Open an [issue](https://github.com/Kylne700/zcrowd/issues/new). Give the expected and the actual behavior, the steps that
reproduce it, and what you collected above. A reduced test case is best. A bug that cannot be reproduced cannot be fixed.

If you want to send a fix, say so in the issue first. Be ready to change your approach after review.

## Reporting Security Issues

Do not report security problems in a public issue, a pull request or a discussion.

Report them privately through a GitHub security advisory on this repository:
[https://github.com/Kylne700/zcrowd/security/advisories/new](https://github.com/Kylne700/zcrowd/security/advisories/new).

Security-relevant here means anything that breaks a claim in [docs/PRIVACY.md](docs/PRIVACY.md). For example: data typed by
the user leaves the machine, the network cut of `check`, `report` or `verify` can be bypassed, a block server that lies is
believed, or a release archive does not match its source.

## Suggesting Enhancements

Track enhancement suggestions as [issues](https://github.com/Kylne700/zcrowd/issues).

Before you open one:

- Read the README to see if it is already there.
- Search the issues. If the idea exists, comment there.
- Check the scope. zcrowd computes only what any chain observer can already compute, for your own transaction, on your own
  machine. A feature that sends what the user types to any server, or adds telemetry, will not be accepted.

In the issue, use a clear title and describe the current behavior, the behavior you expect, why it helps most users, and
what you tried instead.

## Styleguides

### Git Usage

The default branch is `master`. Fork the repository, branch from `master` and open the pull request against `master`.

- Commits are discrete semantic changes. Do not leave work-in-progress commits in a pull request.
- Squash fixes and review changes into the commit they belong to before review starts. While someone is reviewing, add
  new commits instead of force-pushing.
- Separate style-only changes (formatting, renames) from changes in behavior, in different commits.
- Open a pull request as a draft until CI passes. Then mark it ready for review.
- If a pull request fixes a bug, add the failing test in a commit before the fix, when that is practical.

The audit (`tests/audit.sh`, check P11, run by the e2e CI job) allows one neutral commit identity in the history. It also
allows no mail address, home path or personal string in any file. So do not put mail addresses in files, not even in
examples. If P11 fails only because of your commit author, say so in the pull request, and the maintainer decides how to
merge it.

### Commit Messages

- The title is short and starts with the area, as in `README: ...`, `deny: ...`, `Web page: ...`, `Release: ...`.
- The body says why the change is needed. A trivial change may skip the body.
- If more than one person is responsible for a commit, add a `Co-authored-by:` trailer for each of them.

### Changelog

The project has no `CHANGELOG.md` yet. Describe every change that a user can see in the pull request text. If you change
what a command prints, the `--json` output, the data-file format or the public API of `zcrowd-core`, update the README and
`docs/PRIVACY.md` in the same commit, so that they stay true.

### Coding Style

The list below is not complete. Breaking it is likely to get a pull request rejected or sent back for changes. Some old
code does not follow every rule. When you change such code, move it toward these rules, and keep that in a separate commit
from any change in behavior.

#### Formatting and Lints

- `rustfmt.toml` sets `max_width = 140` and `use_small_heuristics = "Max"`. Run `cargo fmt` before you commit.
- Clippy runs with warnings as errors: `cargo clippy --all-targets -- -D warnings`. Fix the lint. Add an `allow` only with
  a comment that says why.

#### Type Safety

- Make invalid states hard to build. Keep fields private when a type has an invariant, and give it a constructor that
  returns a `Result` or an `Option`.
- Use an `enum` with meaningful variants instead of a boolean argument or return value.
- Prefer immutable values. Use `mut` only in the narrowest scope.

#### Public API

- Every `pub` item of `zcrowd-core` is public API. Do not make a type `pub` in a private module.
- New and changed public items need complete `///` documentation comments. Say what the item does, what it returns, and
  when it fails.

#### Side Effects

- Write functions that give the same output for the same input where you can.
- Pass a capability as an argument instead of reaching for it. The library reads blocks only through `BlockSource`, so tests
  run on a fake server (see `crates/zcrowd-core/tests/ingest.rs`). Follow that pattern for new network or clock access.
- Use a `for` loop, not `map`, when the body has a side effect.

#### Error Handling

- Return a `Result`. The code uses `anyhow::Result` and adds context with `anyhow::Context`.
- An error message says what went wrong and, where it can, what the user does next (for example `run zcrowd sync again`).
- Do not use `unwrap`, `expect` or `panic!` on input from a file, a server or the user.

#### Data Formats

- A data file carries its version (`FORMAT_VERSION` in `crates/zcrowd-core/src/model.rs`). Reading refuses any other
  version and writes nothing, so that old data cannot give a false answer.
- The `--json` output and the verdict lines are a stable contract. Changing them needs a discussion in an issue first.

#### `unsafe`

Do not add `unsafe` unless there is no safe way. Today it is used for operating system calls in
`crates/zcrowd/src/sandbox.rs` and for one signal call in a test. Every `unsafe` block needs a `// SAFETY:` comment that says
why it is sound.

#### Privacy Rules

zcrowd is a privacy tool, so these rules are checked by `tests/audit.sh` and described in
[docs/PRIVACY.md](docs/PRIVACY.md):

- What the user types is never printed back and never leaves the machine.
- `check`, `report` and `verify` cut their own network before they read their data.
- No file contains a home path or a mail address (P11), and no release archive contains a build-machine path (R1).

## Tests

- A fix or a new feature comes with a test. A bug fix comes with a test that fails without the fix.
- Tests run offline on the committed fixtures. Do not make a test need a live server or the Tor network. `LIVE=1` adds the live
  Tor checks to `tests/audit.sh` and a real sync to `tests/e2e.sh`. Leave it off for normal work.
- The command line tests are in `crates/zcrowd/tests/`. The library tests are in `crates/zcrowd-core/tests/` and in the
  modules.
- If you change the web page, edit `web/index.template.html` or `web/build.mjs`, then run `node web/build.mjs` and commit
  the new `web/index.html`. Check it against the Rust code with `cargo build -p zcrowd && node web/test/parity.mjs`.

Before you open a pull request, run:

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --no-fail-fast
bash tests/e2e.sh
```

`bash tests/e2e.sh` runs the format check, the build, clippy, every test, the real binary on the fixtures and then
`tests/audit.sh`. It prints `E2E OK` when everything passes. It takes a few minutes. It needs Node and a C compiler, and no network once the crates are downloaded.

## Continuous Integration

CI (`.github/workflows/ci.yml`) must pass before a pull request is merged. The one known exception is the commit-identity
check P11 of the audit (see [Git Usage](#git-usage)). CI runs:

- build and test on Linux, macOS and Windows: `cargo build --locked` and `cargo test --locked --no-fail-fast`,
- the sandbox tests on a default Ubuntu 24.04,
- `bash tests/e2e.sh` (format, clippy, tests, the binary on the fixtures, the audit),
- the web page in Chromium, Firefox and WebKit (`node web/test/headless.mjs`),
- `cargo deny --locked check`.

A red job is a bug in the pull request until shown otherwise. If a job fails for a reason that is not your change, say so in
the pull request.

## Licence

zcrowd is released under the MIT licence (see [LICENSE](LICENSE)). By contributing, you agree that your contribution may be
provided under that licence.

## Attribution

This guide is modeled on the
[librustzcash CONTRIBUTING.md](https://github.com/zcash/librustzcash/blob/main/CONTRIBUTING.md), which is itself based on
the template of the [CONTRIBUTING.md](https://contributing.md/) project.
