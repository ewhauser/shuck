# Changelog

## [0.0.16](https://github.com/ewhauser/shuck/compare/v0.0.15...v0.0.16) (2026-04-21)


### Bug Fixes

* **linter:** report unread loop variables in C001 ([#414](https://github.com/ewhauser/shuck/issues/414)) ([14f9c0c](https://github.com/ewhauser/shuck/commit/14f9c0cca929b78b62b568be7cc203efe5968750))
* **linter:** stop flagging mapfile process substitution ([#416](https://github.com/ewhauser/shuck/issues/416)) ([c730cfb](https://github.com/ewhauser/shuck/commit/c730cfb88c99c2e91e7de70a40388798d50a6bd5))
* **linter:** treat self-referential initializers as reads ([#413](https://github.com/ewhauser/shuck/issues/413)) ([8cda0a1](https://github.com/ewhauser/shuck/commit/8cda0a1b4c79bf7f2aa8425444013ba84326dd6b))
* **semantic:** keep or-fallback reachable after conditional exit ([#415](https://github.com/ewhauser/shuck/issues/415)) ([0005f86](https://github.com/ewhauser/shuck/commit/0005f86a7e56d1aed4f8557e1b75b41367b2c2cd))

## [0.0.15](https://github.com/ewhauser/shuck/compare/v0.0.14...v0.0.15) (2026-04-21)


### Bug Fixes

* cargo install instructions ([#408](https://github.com/ewhauser/shuck/issues/408)) ([4029b81](https://github.com/ewhauser/shuck/commit/4029b81af2a1219835832d2e767497ac82216b59))
* **linter:** ignore unused for-loop counters in C001 ([#410](https://github.com/ewhauser/shuck/issues/410)) ([3ff60ec](https://github.com/ewhauser/shuck/commit/3ff60ecfaba660caf32f62fd6c8d5aa14f20082c))
* **linter:** suppress C001 on intentional empty clears ([#409](https://github.com/ewhauser/shuck/issues/409)) ([3740969](https://github.com/ewhauser/shuck/commit/3740969b159f536d204704c123f13b4a68d06427))

## [0.0.14](https://github.com/ewhauser/shuck/compare/v0.0.13...v0.0.14) (2026-04-21)


### Bug Fixes

* **main:** reduce duplicate C001 reports ([#378](https://github.com/ewhauser/shuck/issues/378)) ([d947cd7](https://github.com/ewhauser/shuck/commit/d947cd7ea617ccd108fd7c1509495eae8d9a365b))

## [0.0.13](https://github.com/ewhauser/shuck/compare/v0.0.12...v0.0.13) (2026-04-21)


### Features

* **release:** publish shuck to homebrew tap ([#399](https://github.com/ewhauser/shuck/issues/399)) ([b0d66dd](https://github.com/ewhauser/shuck/commit/b0d66dd4877e80de94a0a3ed14ef4fee4b383ab5))


### Refactor

* remove non-test unwrap-style calls ([#401](https://github.com/ewhauser/shuck/issues/401)) ([6df705b](https://github.com/ewhauser/shuck/commit/6df705b0aeae5a00bb3d7fcc97c09346adff4fd9))

## [0.0.12](https://github.com/ewhauser/shuck/compare/v0.0.11...v0.0.12) (2026-04-21)


### Documentation

* prepare repo for public OSS release ([#392](https://github.com/ewhauser/shuck/issues/392)) ([20f5335](https://github.com/ewhauser/shuck/commit/20f5335ba60cdc1646045d40f1be1c14681047ad))


### Refactor

* remove non-test unwraps ([#397](https://github.com/ewhauser/shuck/issues/397)) ([0b1b78f](https://github.com/ewhauser/shuck/commit/0b1b78f049823669fabd0b0f7ce46a7b78c5a8b6))
* **semantic:** remove deferred function unsafe dereference ([#394](https://github.com/ewhauser/shuck/issues/394)) ([1b4968f](https://github.com/ewhauser/shuck/commit/1b4968fa70dd2ae94d062458655d196c6d9c75e6))

## [0.0.11](https://github.com/ewhauser/shuck/compare/v0.0.10...v0.0.11) (2026-04-21)


### Miscellaneous

* release 0.0.11 ([#388](https://github.com/ewhauser/shuck/issues/388)) ([6593025](https://github.com/ewhauser/shuck/commit/65930257fccd90687651cd7fbb5df173b9401ce5))

## Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This changelog is generated and maintained by [release-please](https://github.com/googleapis/release-please) from [Conventional Commit](https://www.conventionalcommits.org/) messages on `main`. Do not edit it by hand.
