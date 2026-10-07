## [1.0.3](https://github.com/hyperi-io/git-scrub/compare/v1.0.2...v1.0.3) (2026-10-07)


### Bug Fixes

* **ci:** mint the App token from the client ID, not the app ID ([#2](https://github.com/hyperi-io/git-scrub/issues/2)) ([768f3cc](https://github.com/hyperi-io/git-scrub/commit/768f3ccc52eff1975eb2ad8dbeda23b89f0f1e4d)), closes [hyperi-io/hyperi-ci#100](https://github.com/hyperi-io/hyperi-ci/issues/100)
* **ci:** onboard Renovate and move onto current action majors ([#3](https://github.com/hyperi-io/git-scrub/issues/3)) ([762df9f](https://github.com/hyperi-io/git-scrub/commit/762df9fbd1c22ba188a0739b6a1c17c9c75260f2)), closes [hyperi-io/hyperi-ci#100](https://github.com/hyperi-io/hyperi-ci/issues/100)
* repair the release build and publish to the hyperi-io tap ([#8](https://github.com/hyperi-io/git-scrub/issues/8)) ([fff98c8](https://github.com/hyperi-io/git-scrub/commit/fff98c8fde83d74dad1bf8ceee715f7e99218c4d))

## [1.0.2](https://github.com/hyperi-io/git-scrub/compare/v1.0.1...v1.0.2) (2026-05-26)


### Bug Fixes

* **refresh-advisories:** invert patched ranges + populate snapshot with 832 RustSec entries ([c7d7f14](https://github.com/hyperi-io/git-scrub/commit/c7d7f1458638e8332a8f18d7d9d32a7515194b6d))

## [1.0.1](https://github.com/hyperi-io/git-scrub/compare/v1.0.0...v1.0.1) (2026-05-26)


### Bug Fixes

* **release:** add @semantic-release/github so GH Release + assets are published ([12f0976](https://github.com/hyperi-io/git-scrub/commit/12f09763a2c1c8a0cc878cef43f4190ea8eaf4c0))

# 1.0.0 (2026-05-26)


### Bug Fixes

* **audit:** commits_rewritten only counts actual attribution matches, not cosmetic cleanup ([292b575](https://github.com/hyperi-io/git-scrub/commit/292b57511ed8f27b1aadc57ba2cb82fcd60c3c7a))
* broken intra-doc link in BlobRewriter::is_empty ([1774df9](https://github.com/hyperi-io/git-scrub/commit/1774df9e7a8b119ddd873e46796cf416f43c9b95))
* **engine:** advance next_mark past commit and tag marks to prevent collision ([4cf802b](https://github.com/hyperi-io/git-scrub/commit/4cf802b6bffd8bb7bce88b7660d4f9b50f410f19))
* **patterns:** case-insensitive attribution trailer matching ([d18adf2](https://github.com/hyperi-io/git-scrub/commit/d18adf20ae26bc336c840f7a74f095a2cb8d0d63))
* **patterns:** file globs match nested paths anywhere in tree ([a815435](https://github.com/hyperi-io/git-scrub/commit/a8154357fb16f7775de2688a3521f5ea02240c00))
* **release:** build linux-arm64 via rustup + gcc-aarch64-linux-gnu (no Docker) ([fa17ee4](https://github.com/hyperi-io/git-scrub/commit/fa17ee40d18ceeb7b668346ef4e8339bd4edaa7d))
* **release:** indirect HOMEBREW_APP_ID via env so 'if:' can reference it ([b275fca](https://github.com/hyperi-io/git-scrub/commit/b275fca033d44f51f5a0b52b27864a12499f2bef))
* **release:** install conventional-changelog preset + gate archives on release_needed ([be1ab8c](https://github.com/hyperi-io/git-scrub/commit/be1ab8c7c65271ac0570db8286e3ba226397a1cd))
* **release:** install cross via taiki-e/install-action (puts binary on PATH) ([3280943](https://github.com/hyperi-io/git-scrub/commit/32809432791e9e1d51d7bfb88926c53a63ed35bf))
* **test:** race-safe fixture clone + refs/tags prefix for slash-tag checkout ([07e4e02](https://github.com/hyperi-io/git-scrub/commit/07e4e029485a132521f5962a48a6f025203e5c2b))


### Features

* add `audit` umbrella subcommand and --audit synonym flags ([e680510](https://github.com/hyperi-io/git-scrub/commit/e680510a7653f7853ddd974b668465c41d7af39b))
* add ai curate subcommand for working-tree cleanup ([38198aa](https://github.com/hyperi-io/git-scrub/commit/38198aaa4250915182d3d1d39ece1d58c266af78))
* add BlobCache for path-aware blob rewriting ([a7d7d18](https://github.com/hyperi-io/git-scrub/commit/a7d7d185f994a394294b8f6df4abe2176051f56d))
* add LockfileRewriter trait for path-aware blob rewriting ([068f762](https://github.com/hyperi-io/git-scrub/commit/068f76233d157292813f7fdb32e5489a5b101b4c))
* add supply chain advisory schema and bundled snapshot ([5c23738](https://github.com/hyperi-io/git-scrub/commit/5c2373864db5cef027632053cacc3f1521b8a21a))
* add tools/refresh-advisories workspace member with RustSec fetcher ([7d69c35](https://github.com/hyperi-io/git-scrub/commit/7d69c35dddb66c03ee98a74fe202bc2714d575af))
* add version expression matcher (exact / wildcard / semver range) ([dbdbeae](https://github.com/hyperi-io/git-scrub/commit/dbdbeaecca0c00db871395db8ed74b1b5c459ff2))
* bun.lock (text format) supply chain rewriter (parser + unit + synthetic + e2e) ([9db65d1](https://github.com/hyperi-io/git-scrub/commit/9db65d15bf4fc880821ba08dfa6f9bd505b31aa3))
* cargo.lock supply chain rewriter (parser + unit + synthetic + e2e) ([e3e3d85](https://github.com/hyperi-io/git-scrub/commit/e3e3d8597644f8ecd731ec3eab22b7ebc9edff37))
* **cli:** add clean umbrella subcommand for single-pass composition ([4ec031d](https://github.com/hyperi-io/git-scrub/commit/4ec031d057ed30b1c0070aac56b715131df5aa9c))
* **cli:** add supply subcommand (composite/lockfiles/packages/advisories/patterns) ([9833f0e](https://github.com/hyperi-io/git-scrub/commit/9833f0ef2e2e18a5e6e76505f75b1ad89bc4bd72))
* composer.lock supply chain rewriter — final ecosystem (parser + unit + synthetic + e2e) ([61e8d15](https://github.com/hyperi-io/git-scrub/commit/61e8d1525b050deaa0205a32c47d6580dde73bbf))
* **engine:** wire LockfileRewriter through blob mark-cache ([4db4cfb](https://github.com/hyperi-io/git-scrub/commit/4db4cfbb52ebc0efd2922bfe8ffdd799857fcb3f))
* go.sum supply chain rewriter (parser + unit + synthetic + e2e) ([21a2baf](https://github.com/hyperi-io/git-scrub/commit/21a2baf735434bd45ae6e9d40f5af9f155bb09d7))
* npm lockfile rewriter (parser + unit + synthetic + e2e) ([9faf562](https://github.com/hyperi-io/git-scrub/commit/9faf562997101afc92d35542bdd50c1e9cd0c46d))
* pipfile.lock supply chain rewriter (parser + unit + synthetic + e2e) ([2f23eea](https://github.com/hyperi-io/git-scrub/commit/2f23eeaac95b243cf20846a036226f5b45e30672))
* pnpm-lock.yaml supply chain rewriter (parser + unit + synthetic + e2e) ([5ca55b7](https://github.com/hyperi-io/git-scrub/commit/5ca55b76785bce9d1be7c7f970676b9c8c18debf))
* **release:** scaffold installer distribution (install.sh + nFPM + brew + workflow) ([a00d49e](https://github.com/hyperi-io/git-scrub/commit/a00d49ea2c443e4ac0fa0bcc8ed0287b74b272f1))
* **runbook:** emit supply chain summary section after rewrites ([546f1fb](https://github.com/hyperi-io/git-scrub/commit/546f1fbae2a0c66cd9c9f9bce801c122f1a2b5fd))
* **test:** add e2e harness wired to GIT_SCRUB_TEST_REPO ([07c5056](https://github.com/hyperi-io/git-scrub/commit/07c50569485fb4d20469d0cea409127ad38f6224))
* uv + poetry lockfile support (reuses CargoLockRewriter; fixtures + e2e) ([90f26c7](https://github.com/hyperi-io/git-scrub/commit/90f26c76bda6c34612789804d9caa1ccbf620c02))
* yarn.lock (classic v1) supply chain rewriter (parser + unit + integration + e2e) ([b2872b6](https://github.com/hyperi-io/git-scrub/commit/b2872b62a6a4749f3872ca2e678599c795a00c3d))
