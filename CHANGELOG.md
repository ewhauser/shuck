# Changelog

## [0.0.34](https://github.com/ewhauser/shuck/compare/v0.0.33...v0.0.34) (2026-05-05)


### Features

* **parser:** support zsh brace_ccl expansions ([#834](https://github.com/ewhauser/shuck/issues/834)) ([96aa400](https://github.com/ewhauser/shuck/commit/96aa40001e2dec2a3be8361cff55dbb104870885))


### Bug Fixes

* **linter:** account for zsh array fanout in word facts ([#837](https://github.com/ewhauser/shuck/issues/837)) ([e14d30c](https://github.com/ewhauser/shuck/commit/e14d30c9308331705c343bb3d0a44a3adf541103))
* **linter:** account for zsh file expansion order ([#839](https://github.com/ewhauser/shuck/issues/839)) ([9361305](https://github.com/ewhauser/shuck/commit/936130556dbce3e1fca9066c46b1813d8e91ffca))
* **linter:** account for zsh glob_subst in loop facts ([#836](https://github.com/ewhauser/shuck/issues/836)) ([5689e88](https://github.com/ewhauser/shuck/commit/5689e889e8f5050c22ba17472ebf165a7498d79d))
* **linter:** allow zsh brace-expanded declaration assignments ([#861](https://github.com/ewhauser/shuck/issues/861)) ([dbd3770](https://github.com/ewhauser/shuck/commit/dbd377053c4f00472139e8aca8c2f51c0247b123))
* **linter:** centralize active glob behavior in facts ([#844](https://github.com/ewhauser/shuck/issues/844)) ([e971618](https://github.com/ewhauser/shuck/commit/e9716181e7475c175f48e6d9b2ae62c578973f4a))
* **linter:** handle zsh brace_ccl in facts ([#842](https://github.com/ewhauser/shuck/issues/842)) ([f42db98](https://github.com/ewhauser/shuck/commit/f42db985c17225a1cd8e648c4b9bfc3a9b1bcceb))
* **linter:** handle zsh option-map commas ([#864](https://github.com/ewhauser/shuck/issues/864)) ([3334e31](https://github.com/ewhauser/shuck/commit/3334e31f210f682f63dec97429ca11b0df7511d0))
* **linter:** honor zsh split state in split facts ([#835](https://github.com/ewhauser/shuck/issues/835)) ([54580f5](https://github.com/ewhauser/shuck/commit/54580f5b6d0a8a0578e8eb9fdef9ac0c6f86663d))
* **linter:** model zsh function arity entrypoints ([#860](https://github.com/ewhauser/shuck/issues/860)) ([a998398](https://github.com/ewhauser/shuck/commit/a9983980d44dc348ae2ca4f66ecca7fb635c4d27))
* **linter:** model zsh octal arithmetic literals ([#845](https://github.com/ewhauser/shuck/issues/845)) ([60c0d84](https://github.com/ewhauser/shuck/commit/60c0d8453439b1acc25a2a8788b0c388c303ae88))
* **linter:** partition indexed array facts by behavior ([#841](https://github.com/ewhauser/shuck/issues/841)) ([aa73d23](https://github.com/ewhauser/shuck/commit/aa73d23aad513e69fe440798d4bb30a811c98bcb))
* **linter:** respect zsh equals in assignment facts ([#838](https://github.com/ewhauser/shuck/issues/838)) ([fb1673e](https://github.com/ewhauser/shuck/commit/fb1673eb0cd9b868b8467084423c6238ee539dd4))
* **linter:** suppress zsh delayed expansion C005 ([#863](https://github.com/ewhauser/shuck/issues/863)) ([b75fd7b](https://github.com/ewhauser/shuck/commit/b75fd7bc80f53957631405feb13b124adfba65b7))
* **linter:** treat zsh config namespaces as consumed ([#862](https://github.com/ewhauser/shuck/issues/862)) ([f91429c](https://github.com/ewhauser/shuck/commit/f91429cd80ac0b1105f0c83f2a20632555adf816))
* **parser:** handle zsh numeric assignments ([#851](https://github.com/ewhauser/shuck/issues/851)) ([7c23270](https://github.com/ewhauser/shuck/commit/7c23270c4ace68d7bf57068b8b9023fc8dc0fa9e))
* **parser:** support upstream zsh function and glob forms ([#831](https://github.com/ewhauser/shuck/issues/831)) ([0be09e7](https://github.com/ewhauser/shuck/commit/0be09e710dacd43afabe42bdc5f8073d47451864))
* **semantic:** cache zsh function option summaries ([#840](https://github.com/ewhauser/shuck/issues/840)) ([cf6cb80](https://github.com/ewhauser/shuck/commit/cf6cb80653f8ae578a7dc79927e0708da5374792))
* **semantic:** handle zsh associative runtime keys ([#849](https://github.com/ewhauser/shuck/issues/849)) ([1394489](https://github.com/ewhauser/shuck/commit/13944898e0a96e71cbf4b140dcb33bc2d87167cc))
* **semantic:** handle zsh regex match state ([#853](https://github.com/ewhauser/shuck/issues/853)) ([cd484e4](https://github.com/ewhauser/shuck/commit/cd484e4196f563874efd1652dd2d82def8d71b09))
* **semantic:** ignore zsh existence probe reads ([#846](https://github.com/ewhauser/shuck/issues/846)) ([7ba3ce8](https://github.com/ewhauser/shuck/commit/7ba3ce87be53cc1fc2eb5f58a1b7db05870d9d63))
* **semantic:** model zparseopts targets ([#859](https://github.com/ewhauser/shuck/issues/859)) ([0be9c62](https://github.com/ewhauser/shuck/commit/0be9c622d49ca6cbfc434f50c8a9ce7068af09d0))
* **semantic:** model zsh always cleanup reachability ([#852](https://github.com/ewhauser/shuck/issues/852)) ([4c57526](https://github.com/ewhauser/shuck/commit/4c5752637744587b8a197d54726d887fdf0a9332))
* **semantic:** model zsh by-name helper operands ([#858](https://github.com/ewhauser/shuck/issues/858)) ([c5b896e](https://github.com/ewhauser/shuck/commit/c5b896e2ffbaf6d41d86ab3d8b855a18ef2ce2fa))
* **semantic:** model zsh pipeline tail scope ([#868](https://github.com/ewhauser/shuck/issues/868)) ([61cca0f](https://github.com/ewhauser/shuck/commit/61cca0f7050f0371373f45e18b80fedd074d23c6))
* **zsh:** honor explicit pattern expansion ([#867](https://github.com/ewhauser/shuck/issues/867)) ([c953f34](https://github.com/ewhauser/shuck/commit/c953f34546e4dc06d29e9103cf517450b589de98))
* **zsh:** recognize integer declarations ([#857](https://github.com/ewhauser/shuck/issues/857)) ([28957e6](https://github.com/ewhauser/shuck/commit/28957e685b5c33a9fd92a7e3d76ce30c35a54bf9))


### Performance

* **linter:** collapse parameter expansion classification into one walk ([#854](https://github.com/ewhauser/shuck/issues/854)) ([ff17a59](https://github.com/ewhauser/shuck/commit/ff17a593044ee72b8f20f71ef9f0ba8ca4d8d898))
* **linter:** hoist array-like name lookup out of word fact loop ([#847](https://github.com/ewhauser/shuck/issues/847)) ([4955e78](https://github.com/ewhauser/shuck/commit/4955e781ebc29e76983bd0b50a388aea3aade2ef))
* **linter:** reuse cached SemanticAnalysis in word fact array fanout ([#848](https://github.com/ewhauser/shuck/issues/848)) ([15b1932](https://github.com/ewhauser/shuck/commit/15b19323575b0e69b856769d2a9b96b2c2176a5e))
* **parser:** box fat WordPart variant payloads ([#869](https://github.com/ewhauser/shuck/issues/869)) ([c865d5a](https://github.com/ewhauser/shuck/commit/c865d5a19f68a81d793b989b369cd1a484e97a4e))
* **parser:** inline ZshOptionState::merge field assignments ([#856](https://github.com/ewhauser/shuck/issues/856)) ([de32d34](https://github.com/ewhauser/shuck/commit/de32d3424978687deed21a02781947c19e825ef8))


### Refactor

* **parser:** make ZshOptionState Copy ([#865](https://github.com/ewhauser/shuck/issues/865)) ([dc5cfce](https://github.com/ewhauser/shuck/commit/dc5cfce9be487db4ef82e9fe90773333c5f3ca44))

## [0.0.33](https://github.com/ewhauser/shuck/compare/v0.0.32...v0.0.33) (2026-05-04)


### Features

* **server:** bootstrap LSP scaffold ([#813](https://github.com/ewhauser/shuck/issues/813)) ([8a8dcac](https://github.com/ewhauser/shuck/commit/8a8dcac29d901fafa55cf9e81f843e8bd681db9b))
* **server:** finish the remaining LSP behavior ([#821](https://github.com/ewhauser/shuck/issues/821)) ([a875ff9](https://github.com/ewhauser/shuck/commit/a875ff992db641f754eb4c7955665899ed1ba41c))
* **server:** implement LSP diagnostics pipeline ([#815](https://github.com/ewhauser/shuck/issues/815)) ([5fa78b0](https://github.com/ewhauser/shuck/commit/5fa78b05e13aa92944d32785d48287ca47859be5))


### Bug Fixes

* **linter:** allow zsh plain array scalar reads ([#809](https://github.com/ewhauser/shuck/issues/809)) ([9fb5557](https://github.com/ewhauser/shuck/commit/9fb5557f635bda2ce251e5ec90350e24295cd71e))
* **linter:** isolate local array history by function ([#799](https://github.com/ewhauser/shuck/issues/799)) ([e4b8be1](https://github.com/ewhauser/shuck/commit/e4b8be184bb0ad4748aed37071b936f27ff7bb24))
* **linter:** stop zsh scalar locals inheriting array refs ([#803](https://github.com/ewhauser/shuck/issues/803)) ([13548e5](https://github.com/ewhauser/shuck/commit/13548e563310cfcdd628b86979db44dfc8a8237d))
* **linter:** suppress zsh option-map arithmetic keys ([#810](https://github.com/ewhauser/shuck/issues/810)) ([c4e739e](https://github.com/ewhauser/shuck/commit/c4e739e483a4d4e20acb7a09215963221121756c))
* **server:** cache resolved project settings ([#823](https://github.com/ewhauser/shuck/issues/823)) ([73ce2a4](https://github.com/ewhauser/shuck/commit/73ce2a47efa12032b6bbf62d4cd2ad231590614e))


### Documentation

* specify option-sensitive facts ([#817](https://github.com/ewhauser/shuck/issues/817)) ([d7617a3](https://github.com/ewhauser/shuck/commit/d7617a3234e69d58e89839ec26f8ec339df79dbf))
* **website:** add editor integration guide ([#824](https://github.com/ewhauser/shuck/issues/824)) ([658eec5](https://github.com/ewhauser/shuck/commit/658eec5ebb41e215d7bdfb3f7094993e50accc0f))


### Refactor

* **config:** extract shared shuck-config crate ([#814](https://github.com/ewhauser/shuck/issues/814)) ([cfa1e98](https://github.com/ewhauser/shuck/commit/cfa1e98ab98e8f279bf0d7905df6cf5d9c411b13))
* **linter:** add remaining option-sensitive facts ([#825](https://github.com/ewhauser/shuck/issues/825)) ([5a8f3b4](https://github.com/ewhauser/shuck/commit/5a8f3b4348c3d9c091ead773aa90c0bd3d62945c))
* **linter:** deny wildcard enum matches in rules ([#820](https://github.com/ewhauser/shuck/issues/820)) ([9dae7e2](https://github.com/ewhauser/shuck/commit/9dae7e20a6b39b46c43d9ba49546facb2253a18c))
* **linter:** finish option-sensitive glob behavior migration ([#822](https://github.com/ewhauser/shuck/issues/822)) ([6900e1a](https://github.com/ewhauser/shuck/commit/6900e1ade4f90d31e916684b2630b2ca545518e6))
* **linter:** move C100 array policy into facts ([#819](https://github.com/ewhauser/shuck/issues/819)) ([276ebb5](https://github.com/ewhauser/shuck/commit/276ebb5a6dca66bf446834fa8b754afe6747b520))
* **semantic:** add option-sensitive behavior query ([#818](https://github.com/ewhauser/shuck/issues/818)) ([91692a5](https://github.com/ewhauser/shuck/commit/91692a549d0243d0b99bfd667deea8545dd22c38))

## [0.0.32](https://github.com/ewhauser/shuck/compare/v0.0.31...v0.0.32) (2026-05-03)


### Features

* **website:** add real-world repo benchmarks ([#800](https://github.com/ewhauser/shuck/issues/800)) ([c344ed3](https://github.com/ewhauser/shuck/commit/c344ed3e828768dd90a7ce5822e05a2940397b81))


### Bug Fixes

* **parser:** parse unbraced zsh subscripts ([#796](https://github.com/ewhauser/shuck/issues/796)) ([653d2fa](https://github.com/ewhauser/shuck/commit/653d2faebf7042be9f304923e4c2ce200367cad1))
* **parser:** parse zsh $+name subscripts ([#798](https://github.com/ewhauser/shuck/issues/798)) ([fb00399](https://github.com/ewhauser/shuck/commit/fb0039930dc815db467ad4c21cd8889e667b9c6c))
* **semantic:** model zsh predefined runtime names ([#795](https://github.com/ewhauser/shuck/issues/795)) ([c54a327](https://github.com/ewhauser/shuck/commit/c54a3273dc44e4ce93bce2a90772f76d1ad7a292))


### Documentation

* **specs:** add 018 language server spec ([#797](https://github.com/ewhauser/shuck/issues/797)) ([af039f4](https://github.com/ewhauser/shuck/commit/af039f48ee8330c57abdfa66b9b83eb3ccfde421))

## [0.0.31](https://github.com/ewhauser/shuck/compare/v0.0.30...v0.0.31) (2026-05-02)


### Features

* **cli:** add google named rule selector ([#785](https://github.com/ewhauser/shuck/issues/785)) ([d6cb49f](https://github.com/ewhauser/shuck/commit/d6cb49f67cc0b6d28c1389f4d86fe32c0101811f))
* **run:** add gbash and bashkit runtimes ([#791](https://github.com/ewhauser/shuck/issues/791)) ([c64e81d](https://github.com/ewhauser/shuck/commit/c64e81d0cd0549a6a00f2d255ba5aec58b36b58c))
* **run:** add managed shell runtime commands ([#787](https://github.com/ewhauser/shuck/issues/787)) ([8b2731e](https://github.com/ewhauser/shuck/commit/8b2731e7f3589310d2f50bd059c5e111bc41ee55))
* **run:** support BusyBox on Linux ([#792](https://github.com/ewhauser/shuck/issues/792)) ([01beeae](https://github.com/ewhauser/shuck/commit/01beeae4c8060968edb8d6ef0a8d4c9e2ea20bfa))


### Bug Fixes

* **run:** support shell registry manifests ([#789](https://github.com/ewhauser/shuck/issues/789)) ([4a09e4f](https://github.com/ewhauser/shuck/commit/4a09e4ff82ba19907ae431754e3e57e6226f1878))


### Documentation

* **rules:** spec Google Shell Style rules and stub metadata ([#784](https://github.com/ewhauser/shuck/issues/784)) ([84556b4](https://github.com/ewhauser/shuck/commit/84556b498747876de6d5f1cb11dfa5aa7634bbcd))
* **website:** add shuck run guide ([#793](https://github.com/ewhauser/shuck/issues/793)) ([ede00a9](https://github.com/ewhauser/shuck/commit/ede00a958e848b6bb836cdc8213292e55abda350))

## [0.0.30](https://github.com/ewhauser/shuck/compare/v0.0.29...v0.0.30) (2026-05-02)


### Bug Fixes

* **cli:** align full-output diagnostic highlights ([#782](https://github.com/ewhauser/shuck/issues/782)) ([3bbad9e](https://github.com/ewhauser/shuck/commit/3bbad9ef674a10bfb3843e565f654fc02f23e45c))

## [0.0.29](https://github.com/ewhauser/shuck/compare/v0.0.28...v0.0.29) (2026-04-30)


### Performance

* **linter:** trim possible-variable-misspelling lookup hotspots ([#777](https://github.com/ewhauser/shuck/issues/777)) ([f7dd4b9](https://github.com/ewhauser/shuck/commit/f7dd4b9c20431278a4f08e36de0103cb2b0fc557))

## [0.0.28](https://github.com/ewhauser/shuck/compare/v0.0.27...v0.0.28) (2026-04-30)


### Performance

* **linter:** binary-search zsh option snapshots, skip ASCII smart-quote scan ([#774](https://github.com/ewhauser/shuck/issues/774)) ([333e829](https://github.com/ewhauser/shuck/commit/333e82922f070acc37676c91008e711d75610ed3))
* **linter:** cut three lint-time hotspots on large zsh files ([#771](https://github.com/ewhauser/shuck/issues/771)) ([d8c656c](https://github.com/ewhauser/shuck/commit/d8c656cdc4d425977a039d8319775a7f1169fc1b))
* **linter:** drop densify-then-compact pass in LinterFactsBuilder ([#753](https://github.com/ewhauser/shuck/issues/753)) ([3933742](https://github.com/ewhauser/shuck/commit/39337420bd2cf7220cfa26f41d4716a39a7ac288))
* **linter:** index assignment-value target spans for misspelling rule ([#769](https://github.com/ewhauser/shuck/issues/769)) ([8ba2b4a](https://github.com/ewhauser/shuck/commit/8ba2b4ac4d575c5f1b6d75fc79c0746305da43d1))
* **linter:** precompute pending-until depths for parse-diagnostic checks ([#772](https://github.com/ewhauser/shuck/issues/772)) ([03ce0c5](https://github.com/ewhauser/shuck/commit/03ce0c5786cedde4ab4c4195cb6f4c8984ba2921))
* **linter:** scan command-leading words at byte level ([#770](https://github.com/ewhauser/shuck/issues/770)) ([e0216e2](https://github.com/ewhauser/shuck/commit/e0216e25094478a1ffea09caa9454c1f17ee42ab))
* **linter:** stream parse-diagnostic shell-like words ([#768](https://github.com/ewhauser/shuck/issues/768)) ([51decde](https://github.com/ewhauser/shuck/commit/51decde79f9583b191e78902168204fb3abade25))
* **linter:** use line index for parse-diagnostic span lookups ([#765](https://github.com/ewhauser/shuck/issues/765)) ([98bb635](https://github.com/ewhauser/shuck/commit/98bb635bbaa784b7fc2dae4ba213ba8eb737e41c))
* **parser:** watermark append-only fields in ParserCheckpoint ([#755](https://github.com/ewhauser/shuck/issues/755)) ([d7736af](https://github.com/ewhauser/shuck/commit/d7736afcc439ed64ba868ceae6219d575f821905))


### Refactor

* **linter:** add semantic command topology ([#764](https://github.com/ewhauser/shuck/issues/764)) ([467e29d](https://github.com/ewhauser/shuck/commit/467e29d3dd0279b24ff9845465a3c583fbfd99a6))
* **linter:** consolidate fact topology helpers ([#766](https://github.com/ewhauser/shuck/issues/766)) ([7622a77](https://github.com/ewhauser/shuck/commit/7622a7708a29861b8fd9bba05bd7f11838c1f1d3))
* **linter:** consolidate offset-to-position lookups behind Locator ([#767](https://github.com/ewhauser/shuck/issues/767)) ([719de41](https://github.com/ewhauser/shuck/commit/719de41dc010c0cbfe500852502db8b91766c0bb))
* **linter:** internalize directive parsing seam ([#756](https://github.com/ewhauser/shuck/issues/756)) ([e0168ce](https://github.com/ewhauser/shuck/commit/e0168ce3f0d0dd6d47bb6355ce1df09e1bacf231))
* **linter:** remove recursive traversal test harness ([#758](https://github.com/ewhauser/shuck/issues/758)) ([37287df](https://github.com/ewhauser/shuck/commit/37287df4a31d4b48cd9596b16fc67e021d6765d5))
* **linter:** remove substitution body walks ([#760](https://github.com/ewhauser/shuck/issues/760)) ([a16029b](https://github.com/ewhauser/shuck/commit/a16029bcd584539a8bffbc08c2e1c8ffe315408b))
* **linter:** remove suppression fallback walk ([#754](https://github.com/ewhauser/shuck/issues/754)) ([a787a72](https://github.com/ewhauser/shuck/commit/a787a72f07080dafefea03cf4467bf59526467e6))
* **linter:** reuse command stream for conditional fact scans ([#759](https://github.com/ewhauser/shuck/issues/759)) ([92210da](https://github.com/ewhauser/shuck/commit/92210da2cde1f40fa65ebf7e9321af024d6d6b95))
* **linter:** reuse semantic conditional traversal ([#761](https://github.com/ewhauser/shuck/issues/761)) ([28d6ca9](https://github.com/ewhauser/shuck/commit/28d6ca9f79bade240947b09f95bf069a2cc97f24))
* **linter:** reuse semantic visits for base prefix facts ([#762](https://github.com/ewhauser/shuck/issues/762)) ([7e5a8ef](https://github.com/ewhauser/shuck/commit/7e5a8efe456ab0366e035428b33304677717cc70))
* **linter:** reuse semantic visits for parse diagnostics ([#763](https://github.com/ewhauser/shuck/issues/763)) ([4e80dfa](https://github.com/ewhauser/shuck/commit/4e80dfaf4d8ea7887bdb5ff7a551e7d3d550fa8e))
* **linter:** reuse semantic walk for directive attachment ([#757](https://github.com/ewhauser/shuck/issues/757)) ([7eed58d](https://github.com/ewhauser/shuck/commit/7eed58dc9d53a3088d51925fb76a6d237e8047a6))

## [0.0.27](https://github.com/ewhauser/shuck/compare/v0.0.26...v0.0.27) (2026-04-29)


### Performance

* **ast:** single-pass Position::advanced_by ([#750](https://github.com/ewhauser/shuck/issues/750)) ([d71e87d](https://github.com/ewhauser/shuck/commit/d71e87da075cd7f67c21aaa97b5cd5c22417332a))
* cut ~18% of linter allocations on large fixtures ([#745](https://github.com/ewhauser/shuck/issues/745)) ([2a8898b](https://github.com/ewhauser/shuck/commit/2a8898bce0e75e7a443effd860b7b35b40927de0))
* **linter:** cut facts allocation blocks with SmallVec and BitVec ([#751](https://github.com/ewhauser/shuck/issues/751)) ([a75ca4d](https://github.com/ewhauser/shuck/commit/a75ca4d8941e5252bfbef6705557377feb75934f))
* **linter:** reuse semantic visits for substitution candidates ([#737](https://github.com/ewhauser/shuck/issues/737)) ([c21ce95](https://github.com/ewhauser/shuck/commit/c21ce9528afc8a2a57e62a901652b91625f0ea99))
* **parser:** short-circuit pure-literal source-backed words ([#748](https://github.com/ewhauser/shuck/issues/748)) ([550e588](https://github.com/ewhauser/shuck/commit/550e58878eaa717a88c5f42edfe8a3c093a84b32))
* **parser:** skip zsh glob word probe on non-zsh Word tokens ([#749](https://github.com/ewhauser/shuck/issues/749)) ([c6080d3](https://github.com/ewhauser/shuck/commit/c6080d39cb3ba2c596d2361973a8d6531f104986))


### Documentation

* **semantic:** document shuck-semantic public API ([#744](https://github.com/ewhauser/shuck/issues/744)) ([a4cf093](https://github.com/ewhauser/shuck/commit/a4cf09351b9c24150b8dbe9c6f881e459c5d4d15))


### Refactor

* **linter:** fuse smart-quote scan and trim capacity estimate ([#747](https://github.com/ewhauser/shuck/issues/747)) ([a334695](https://github.com/ewhauser/shuck/commit/a334695d256f696b3d50db408c68032e53ba2acc))
* **linter:** remove stale dead code ([#738](https://github.com/ewhauser/shuck/issues/738)) ([4e5586b](https://github.com/ewhauser/shuck/commit/4e5586b51ea74c57c5f3f9f802f4b98233e67919))
* remove remaining dead code suppressions ([#739](https://github.com/ewhauser/shuck/issues/739)) ([26b7e01](https://github.com/ewhauser/shuck/commit/26b7e01db88dfe0b343b53610bc644b632d2ffd7))
* **semantic:** extract call payload grouping ([#742](https://github.com/ewhauser/shuck/issues/742)) ([eae9632](https://github.com/ewhauser/shuck/commit/eae9632c9996cebf7bb3418b270d8f3c1acb2f5f))
* **semantic:** own case CLI reachability ([#743](https://github.com/ewhauser/shuck/issues/743)) ([aaabb15](https://github.com/ewhauser/shuck/commit/aaabb15044f76c2df5a8691ccc884ecaac98c123))
* **semantic:** reuse function scope index ([#741](https://github.com/ewhauser/shuck/issues/741)) ([6354af6](https://github.com/ewhauser/shuck/commit/6354af6d37ea42a984f417cd8b75e04bd67384c8))
* **semantic:** reuse lexical function lookup ([#740](https://github.com/ewhauser/shuck/issues/740)) ([a370b36](https://github.com/ewhauser/shuck/commit/a370b36a6501f420488946f84affffb92d7064d0))

## [0.0.26](https://github.com/ewhauser/shuck/compare/v0.0.25...v0.0.26) (2026-04-28)


### Bug Fixes

* **linter:** reduce S001 reviewed divergences ([#674](https://github.com/ewhauser/shuck/issues/674)) ([b1e790e](https://github.com/ewhauser/shuck/commit/b1e790e83bc1f00e45158fcce2dd26f90d71ead8))
* **semantic:** exclude synthetic ids from public commands iteration ([#689](https://github.com/ewhauser/shuck/issues/689)) ([51a745c](https://github.com/ewhauser/shuck/commit/51a745c87193f503b94b8a1926f7e65ce5f613b3))
* **semantic:** resolve alias function flow ([#714](https://github.com/ewhauser/shuck/issues/714)) ([a868986](https://github.com/ewhauser/shuck/commit/a868986a70b2b3119de02d7c73780d4e7dc4de36))


### Performance

* **ast:** add ASCII fast path to Position::advanced_by ([#718](https://github.com/ewhauser/shuck/issues/718)) ([639c717](https://github.com/ewhauser/shuck/commit/639c7174c0a429eaa929d9b5f081b4b1ca6029ea))
* **linter:** binary-search commands contained in pipeline span ([#723](https://github.com/ewhauser/shuck/issues/723)) ([242b837](https://github.com/ewhauser/shuck/commit/242b837746fe7883c9f11600f286c56685651f96))
* **linter:** borrow list segment assignment target from source ([#721](https://github.com/ewhauser/shuck/issues/721)) ([9d1c424](https://github.com/ewhauser/shuck/commit/9d1c424f047a7907e098bd72752dfde914659e7c))
* **linter:** borrow pipeline segment names from source ([#719](https://github.com/ewhauser/shuck/issues/719)) ([d70e7f5](https://github.com/ewhauser/shuck/commit/d70e7f506bc672f3ece5d75a54dcb9c0d7d21617))
* **linter:** drop redundant command-fact source-order scan ([#702](https://github.com/ewhauser/shuck/issues/702)) ([c729eaa](https://github.com/ewhauser/shuck/commit/c729eaadcdf0a6bc3691eca87fe7edef57399944))
* **linter:** index suppression command spans once per file ([#686](https://github.com/ewhauser/shuck/issues/686)) ([cdcf89e](https://github.com/ewhauser/shuck/commit/cdcf89e6abebe7ce5eda144bd0dd9495587875b3))
* **linter:** reuse semantic body indexes for arithmetic scans ([#734](https://github.com/ewhauser/shuck/issues/734)) ([e1dcca2](https://github.com/ewhauser/shuck/commit/e1dcca26b6d2cb9420b0778bdb38db933ac71076))
* **linter:** reuse semantic command body indexes ([#733](https://github.com/ewhauser/shuck/issues/733)) ([1f74c9c](https://github.com/ewhauser/shuck/commit/1f74c9c537355de337333892e3f9d19c3a3a2ed5))
* **linter:** tighten array-assignment split scalar expansion scan ([#708](https://github.com/ewhauser/shuck/issues/708)) ([964db94](https://github.com/ewhauser/shuck/commit/964db94e394b6c54a32d57a128eb949e079636fb))
* **semantic:** avoid condition context rescans ([#731](https://github.com/ewhauser/shuck/issues/731)) ([388c9d5](https://github.com/ewhauser/shuck/commit/388c9d5bf902a93cade0dfb5c16bf97ba11a127c))
* **semantic:** cache function-definition bindings index ([#685](https://github.com/ewhauser/shuck/issues/685)) ([1b51fdf](https://github.com/ewhauser/shuck/commit/1b51fdf59547f68ba8012bb9f4aa1b3483fc3994))
* **semantic:** hoist escaped-template scan to per-word ([#716](https://github.com/ewhauser/shuck/issues/716)) ([495cae1](https://github.com/ewhauser/shuck/commit/495cae17375a5abbef27af7754d676125adb710b))
* **semantic:** index callees by enclosing function in call graph BFS ([#715](https://github.com/ewhauser/shuck/issues/715)) ([0c5c7e3](https://github.com/ewhauser/shuck/commit/0c5c7e3f9e1697544c14ecd5b35ce83d423c6904))
* **semantic:** use dense visited bitset for cfg reachability DFS ([#713](https://github.com/ewhauser/shuck/issues/713)) ([35b370e](https://github.com/ewhauser/shuck/commit/35b370e1a86ca5d617fbc32bee9b31e8fc6ae140))


### Documentation

* **indexer:** document public API contracts ([#707](https://github.com/ewhauser/shuck/issues/707)) ([605bba5](https://github.com/ewhauser/shuck/commit/605bba5911e460f24d6d193f081ab7e0b05733cb))
* **parser:** document public API surface ([#705](https://github.com/ewhauser/shuck/issues/705)) ([1a5ff2b](https://github.com/ewhauser/shuck/commit/1a5ff2b2c8ae8d19997625036ac15ade0e54d558))


### Refactor

* **linter:** consolidate command substitution word traversal ([#697](https://github.com/ewhauser/shuck/issues/697)) ([d0c4c74](https://github.com/ewhauser/shuck/commit/d0c4c745a720424790e601b78a86e86741d96702))
* **linter:** remove checker AST accessor ([#732](https://github.com/ewhauser/shuck/issues/732)) ([ae6050e](https://github.com/ewhauser/shuck/commit/ae6050e4100d1aaabda3950b28c30c0d8b9e56b6))
* **linter:** reuse command topology facts ([#701](https://github.com/ewhauser/shuck/issues/701)) ([8972c79](https://github.com/ewhauser/shuck/commit/8972c79c7ed3dfcc82645e7d5f5eab648203fd0e))
* **linter:** reuse semantic command child index ([#727](https://github.com/ewhauser/shuck/issues/727)) ([b02fd74](https://github.com/ewhauser/shuck/commit/b02fd744e83348dd28fbf0b5c3bd3ae5ca1618df))
* **linter:** reuse semantic function scope checks ([#698](https://github.com/ewhauser/shuck/issues/698)) ([3be31d9](https://github.com/ewhauser/shuck/commit/3be31d9be51e590a35113f0d7882bb0a1dbad430))
* **linter:** reuse semantic function scope lookup ([#709](https://github.com/ewhauser/shuck/issues/709)) ([53e8232](https://github.com/ewhauser/shuck/commit/53e8232723064d91f3f1e3047090601cfded2716))
* **linter:** reuse semantic function scope lookup ([#710](https://github.com/ewhauser/shuck/issues/710)) ([bd9439c](https://github.com/ewhauser/shuck/commit/bd9439c3e2580f2a349e6b785680c96035b821a4))
* **linter:** reuse semantic reference span lookup ([#690](https://github.com/ewhauser/shuck/issues/690)) ([14552b5](https://github.com/ewhauser/shuck/commit/14552b5f1eaf41ce07f5f9480674972595c4000c))
* **linter:** reuse semantic reference span lookups ([#695](https://github.com/ewhauser/shuck/issues/695)) ([ae11246](https://github.com/ewhauser/shuck/commit/ae11246f419453cbcddd87b6a63322c72de8a00c))
* **linter:** share binding visibility helpers ([#703](https://github.com/ewhauser/shuck/issues/703)) ([01242ce](https://github.com/ewhauser/shuck/commit/01242ce7603923df2433cb2e9ebffeb4f6f19a3b))
* **semantic:** centralize assoc binding lookup ([#691](https://github.com/ewhauser/shuck/issues/691)) ([12b1dbd](https://github.com/ewhauser/shuck/commit/12b1dbdeb514224d3da9559792f3c2b0fa162a21))
* **semantic:** centralize function call resolution ([#694](https://github.com/ewhauser/shuck/issues/694)) ([f7dda8d](https://github.com/ewhauser/shuck/commit/f7dda8d2cc96b441790659edb06ab8c44362eced))
* **semantic:** centralize scope predicates ([#700](https://github.com/ewhauser/shuck/issues/700)) ([84fd1ae](https://github.com/ewhauser/shuck/commit/84fd1ae6fa690a0b81ab6a7c0df1bc1895351ec3))
* **semantic:** centralize transient scope boundaries ([#712](https://github.com/ewhauser/shuck/issues/712)) ([bdf5afd](https://github.com/ewhauser/shuck/commit/bdf5afdb2bcafe129a987f773b4e583175bb3772))
* **semantic:** consolidate CFG reachability traversal ([#693](https://github.com/ewhauser/shuck/issues/693)) ([3e98f75](https://github.com/ewhauser/shuck/commit/3e98f759031888f106a60fa045230ac8a2a8003c))
* **semantic:** consolidate enclosing function scope lookup ([#711](https://github.com/ewhauser/shuck/issues/711)) ([e4b6160](https://github.com/ewhauser/shuck/commit/e4b616048d3799d773434b43930fedabcb7565ac))
* **semantic:** expose function binding lookup ([#692](https://github.com/ewhauser/shuck/issues/692)) ([295702b](https://github.com/ewhauser/shuck/commit/295702bd3d89a0549fd64a76533359c9e51cc697))
* **semantic:** expose nested function scope query ([#729](https://github.com/ewhauser/shuck/issues/729)) ([36517f6](https://github.com/ewhauser/shuck/commit/36517f674486d2859dd935c15e0aaed95270f9f1))
* **semantic:** expose visible candidate bindings ([#696](https://github.com/ewhauser/shuck/issues/696)) ([979cc52](https://github.com/ewhauser/shuck/commit/979cc523deddac31e0941e388e207553cb5da102))
* **semantic:** extract safe value flow queries ([#724](https://github.com/ewhauser/shuck/issues/724)) ([c3ec9fa](https://github.com/ewhauser/shuck/commit/c3ec9fa9533d703577aca8c65df35eec9707490d))
* **semantic:** index bindings by definition span ([#726](https://github.com/ewhauser/shuck/issues/726)) ([2624d2b](https://github.com/ewhauser/shuck/commit/2624d2b43831b5476026f1816131aa2c4bfb3849))
* **semantic:** index command contexts for linter facts ([#730](https://github.com/ewhauser/shuck/issues/730)) ([6932536](https://github.com/ewhauser/shuck/commit/6932536c0a6c7b0dabecb4a6fcb6f942e76321ae))
* **semantic:** move command containment queries out of linter ([#720](https://github.com/ewhauser/shuck/issues/720)) ([52d0a01](https://github.com/ewhauser/shuck/commit/52d0a012e5aa1111c274166f91e67d092da75309))
* **semantic:** move env-prefix reference queries into semantic ([#717](https://github.com/ewhauser/shuck/issues/717)) ([952c7e9](https://github.com/ewhauser/shuck/commit/952c7e9e3eefb4824b6504fff12e8831da4cc059))
* **semantic:** move reference summary queries out of linter ([#722](https://github.com/ewhauser/shuck/issues/722)) ([a8ec3b6](https://github.com/ewhauser/shuck/commit/a8ec3b67379f8e7fc708185976aa09bcdac3b47c))
* **semantic:** move safe-value flow queries ([#706](https://github.com/ewhauser/shuck/issues/706)) ([6750771](https://github.com/ewhauser/shuck/commit/6750771b6264abc13ff88fa0ab9457b2b6aadff2))
* **semantic:** own command topology ([#687](https://github.com/ewhauser/shuck/issues/687)) ([c53dfb2](https://github.com/ewhauser/shuck/commit/c53dfb2fe8e4c3f92f38de9f4fce99137cacb50e))
* **semantic:** own function call reachability ([#704](https://github.com/ewhauser/shuck/issues/704)) ([49b23a6](https://github.com/ewhauser/shuck/commit/49b23a63671095d4492ee21662f730ff6427b566))
* **semantic:** share resolved function call scope lookup ([#728](https://github.com/ewhauser/shuck/issues/728)) ([b297c42](https://github.com/ewhauser/shuck/commit/b297c4280559223e80b60631d1038ca8383be91a))

## [0.0.25](https://github.com/ewhauser/shuck/compare/v0.0.24...v0.0.25) (2026-04-27)


### Bug Fixes

* **linter:** add autofix for redundant echo spaces ([#680](https://github.com/ewhauser/shuck/issues/680)) ([2548cc7](https://github.com/ewhauser/shuck/commit/2548cc7d90ae2fa9dbc49771ed178e00991125af))
* **semantic:** share function call binding resolution ([#675](https://github.com/ewhauser/shuck/issues/675)) ([f157215](https://github.com/ewhauser/shuck/commit/f1572152a00536377d5cfc1315011a469e170b39))


### Performance

* **linter:** add simple-glob fast path for case-pattern matcher ([#663](https://github.com/ewhauser/shuck/issues/663)) ([8ea3736](https://github.com/ewhauser/shuck/commit/8ea373654e8dcf60bdbfbfa05b68775453f758f9))
* **linter:** bracket nested-scope walks by command index ([#643](https://github.com/ewhauser/shuck/issues/643)) ([6c73acd](https://github.com/ewhauser/shuck/commit/6c73acda63bac51bdf4bb5b924b57d65476f7ba4))
* **linter:** collapse safe_value into S001 ([#647](https://github.com/ewhauser/shuck/issues/647)) ([df6423b](https://github.com/ewhauser/shuck/commit/df6423b605f61f698f24a7aaf9d0c7e8d691a811))
* **linter:** index function call sites via semantic call graph ([#649](https://github.com/ewhauser/shuck/issues/649)) ([7276273](https://github.com/ewhauser/shuck/commit/7276273d9aa810b1b4ed66cc6de2d99ca0ac4310))
* **linter:** reuse command-offset order in presence facts ([#672](https://github.com/ewhauser/shuck/issues/672)) ([fbe6c7a](https://github.com/ewhauser/shuck/commit/fbe6c7aa109a146ad8ce81068b533cc80c3130f2))
* **linter:** reuse semantic analysis for facts ([#667](https://github.com/ewhauser/shuck/issues/667)) ([80354fb](https://github.com/ewhauser/shuck/commit/80354fb61118aeccd0139cda7d3a1a14fe168630))
* **linter:** reuse semantic reference span index ([#662](https://github.com/ewhauser/shuck/issues/662)) ([c988df0](https://github.com/ewhauser/shuck/commit/c988df02d6ea149a2cc113b5748ad9d5440f9901))
* **linter:** skip array-split scan without command substitutions ([#638](https://github.com/ewhauser/shuck/issues/638)) ([856b673](https://github.com/ewhauser/shuck/commit/856b6739a066ec7cb1608cfaf381c8e54957d63e))
* **linter:** speed up facts builder hotspots ([#659](https://github.com/ewhauser/shuck/issues/659)) ([df0d258](https://github.com/ewhauser/shuck/commit/df0d258d60f424a5e2147b1deb3e7f3e9f5f05af))
* **linter:** speed up local-cross-reference rule ([#650](https://github.com/ewhauser/shuck/issues/650)) ([6699e42](https://github.com/ewhauser/shuck/commit/6699e42ab3fc1f0c7fc23ee2cc59a8a0f2e9266a))
* **linter:** u128 bitset NFA for case-pattern matcher ([#666](https://github.com/ewhauser/shuck/issues/666)) ([3a263b3](https://github.com/ewhauser/shuck/commit/3a263b320e70af5d54395709ca60fc90cba90b46))
* **semantic:** cache binding-block index for reachability queries ([#657](https://github.com/ewhauser/shuck/issues/657)) ([ecee0f0](https://github.com/ewhauser/shuck/commit/ecee0f08fd08568f19fd85d825d0d8755484ea52))
* **semantic:** speed up exact unused assignments ([#642](https://github.com/ewhauser/shuck/issues/642)) ([4ce6854](https://github.com/ewhauser/shuck/commit/4ce6854fc4a9ac84392721093c71580a87f029ef))
* **semantic:** use line-start index in source_line ([#681](https://github.com/ewhauser/shuck/issues/681)) ([e74288c](https://github.com/ewhauser/shuck/commit/e74288cf376434d15a4acb2918ae06ddb5638420))


### Refactor

* **cli:** split check command modules ([#656](https://github.com/ewhauser/shuck/issues/656)) ([1c45d9b](https://github.com/ewhauser/shuck/commit/1c45d9b2db7268f4db1ee7423c8fdf88f0320a5c))
* **linter:** add profiler frames for fact building ([#636](https://github.com/ewhauser/shuck/issues/636)) ([16a7538](https://github.com/ewhauser/shuck/commit/16a7538f5934f2d21c49e50ec075f35d367b6390))
* **linter:** move safe-value flow helpers to semantic ([#644](https://github.com/ewhauser/shuck/issues/644)) ([82d4619](https://github.com/ewhauser/shuck/commit/82d46196a2f36bcc23493aecdb99a63816043c1e))
* **linter:** reuse semantic list and pipeline shapes ([#679](https://github.com/ewhauser/shuck/issues/679)) ([124ac18](https://github.com/ewhauser/shuck/commit/124ac18549f2f609da33905d267aef5616947d6e))
* **linter:** reuse semantic statement sequences ([#677](https://github.com/ewhauser/shuck/issues/677)) ([83f7e56](https://github.com/ewhauser/shuck/commit/83f7e56dea72ae5f9cb2d878a36e8ef523b7ddb6))
* **linter:** share overwritten function analysis ([#651](https://github.com/ewhauser/shuck/issues/651)) ([dbaf467](https://github.com/ewhauser/shuck/commit/dbaf4672ad2b33dff952989f85c7138060d5650b))
* **linter:** split command option facts ([#645](https://github.com/ewhauser/shuck/issues/645)) ([4032284](https://github.com/ewhauser/shuck/commit/40322849494dce1a7f1272f3c3fd4a0725c1c997))
* **linter:** split word facts module ([#640](https://github.com/ewhauser/shuck/issues/640)) ([ad8d476](https://github.com/ewhauser/shuck/commit/ad8d476eaae64440b6caf95540ca946dc1b56f28))
* **linter:** split word span facts ([#653](https://github.com/ewhauser/shuck/issues/653)) ([06cc626](https://github.com/ewhauser/shuck/commit/06cc626004b7164ed5059f462a549948c4ff72ba))
* **parser:** narrow public API surface ([#682](https://github.com/ewhauser/shuck/issues/682)) ([2676e5e](https://github.com/ewhauser/shuck/commit/2676e5e141fa3fb718dd5eaa08ae9813149f3c9e))
* **parser:** split parser module internals ([#654](https://github.com/ewhauser/shuck/issues/654)) ([6fd5fa1](https://github.com/ewhauser/shuck/commit/6fd5fa11f6d4dd262b2a15e4e2aec9985ca29aa9))
* **semantic:** expose function binding facts ([#660](https://github.com/ewhauser/shuck/issues/660)) ([ee9231c](https://github.com/ewhauser/shuck/commit/ee9231c922da395bceba730e045309b670240306))
* **semantic:** expose function reachability helpers ([#648](https://github.com/ewhauser/shuck/issues/648)) ([06fa5ae](https://github.com/ewhauser/shuck/commit/06fa5ae98d67b2dd1260507a59488de383256884))
* **semantic:** expose nonpersistent assignment analysis ([#665](https://github.com/ewhauser/shuck/issues/665)) ([63c4c2f](https://github.com/ewhauser/shuck/commit/63c4c2f7e6f9aa263dcafa9545c9ea80cb175257))
* **semantic:** index declarations by command span ([#670](https://github.com/ewhauser/shuck/issues/670)) ([9e108bb](https://github.com/ewhauser/shuck/commit/9e108bb8f32b5aeb1ed12b846a5c94d3a55d6937))
* **semantic:** reuse command normalization for zsh effects ([#678](https://github.com/ewhauser/shuck/issues/678)) ([1987f05](https://github.com/ewhauser/shuck/commit/1987f05a6afbf0eff380052dd03609169238e41d))
* **semantic:** reuse recorded function scopes ([#668](https://github.com/ewhauser/shuck/issues/668)) ([2e3cf91](https://github.com/ewhauser/shuck/commit/2e3cf918d9e76c0f6a11b3b3b9136fa27164517c))
* **semantic:** share ancestor scope traversal ([#676](https://github.com/ewhauser/shuck/issues/676)) ([f3dd66e](https://github.com/ewhauser/shuck/commit/f3dd66ef324eae320d281e403b24aabe634c198e))
* **semantic:** share call graph construction ([#658](https://github.com/ewhauser/shuck/issues/658)) ([466f19b](https://github.com/ewhauser/shuck/commit/466f19b6661f43a9b4577c49c07f6fb645edbcad))
* **semantic:** split semantic builder modules ([#652](https://github.com/ewhauser/shuck/issues/652)) ([2995219](https://github.com/ewhauser/shuck/commit/29952194faf446c9b38ce565e294ab52a56ab059))
* **semantic:** split semantic facade modules ([#646](https://github.com/ewhauser/shuck/issues/646)) ([3b7b7db](https://github.com/ewhauser/shuck/commit/3b7b7dbed55713bfb42290a71b5430e4437b4c24))

## [0.0.24](https://github.com/ewhauser/shuck/compare/v0.0.23...v0.0.24) (2026-04-26)


### Bug Fixes

* **extract:** handle GitHub Actions workflow anchors ([#609](https://github.com/ewhauser/shuck/issues/609)) ([81d3e99](https://github.com/ewhauser/shuck/commit/81d3e9933c9018f23dd7682598fb9f55607c9c48))
* **extract:** parse GitHub Actions YAML with saphyr ([#615](https://github.com/ewhauser/shuck/issues/615)) ([2029aa5](https://github.com/ewhauser/shuck/commit/2029aa537d2a926518735083890f4cb74768bdc9))
* **linter:** ratchet S001 quote exposure parity ([#616](https://github.com/ewhauser/shuck/issues/616)) ([02f0b02](https://github.com/ewhauser/shuck/commit/02f0b02e2940b91d525284b7049b62b611de7640))


### Performance

* **indexer:** fold continuation discovery into line scan ([#623](https://github.com/ewhauser/shuck/issues/623)) ([b607123](https://github.com/ewhauser/shuck/commit/b607123f02b86c0a4739bcc86d18aa769796037e))
* **linter:** cache command scope to elide per-iteration scope_at ([#625](https://github.com/ewhauser/shuck/issues/625)) ([e3c9152](https://github.com/ewhauser/shuck/commit/e3c9152192a5f9ecf0eb37c4c14c9409babaf39a))
* **linter:** index unset commands for safe values ([#633](https://github.com/ewhauser/shuck/issues/633)) ([0d532ee](https://github.com/ewhauser/shuck/commit/0d532eece6c3a4c77f2c9323698525608db1a37b))
* **linter:** specialize scope compat misspelling scan ([#631](https://github.com/ewhauser/shuck/issues/631)) ([4e71dba](https://github.com/ewhauser/shuck/commit/4e71dbac4bb4ecf8da6b54bc730368ea8d1bb24c))
* **linter:** speed up misspelling lookup ([#629](https://github.com/ewhauser/shuck/issues/629)) ([59d676d](https://github.com/ewhauser/shuck/commit/59d676db222ffffbe024771973eb917742d47136))


### Documentation

* **website:** show rule autofix status ([#627](https://github.com/ewhauser/shuck/issues/627)) ([7b36f95](https://github.com/ewhauser/shuck/commit/7b36f95dc97ba4fa9102b83e3972cd033a6df06c))


### Refactor

* **linter:** move status capture values into facts ([#630](https://github.com/ewhauser/shuck/issues/630)) ([1e9c0dc](https://github.com/ewhauser/shuck/commit/1e9c0dceaa9d383001d0b7b9fe770ea545b2f920))
* **linter:** remove file context plumbing ([#622](https://github.com/ewhauser/shuck/issues/622)) ([4a84cfb](https://github.com/ewhauser/shuck/commit/4a84cfbd17723c6b04fa3613403ac4ea1af77cb4))
* **linter:** remove helper library context ([#620](https://github.com/ewhauser/shuck/issues/620)) ([8d2700a](https://github.com/ewhauser/shuck/commit/8d2700a04f1d9613741e72e59b8b969c9e4cbbe8))
* **linter:** remove shellspec context ([#621](https://github.com/ewhauser/shuck/issues/621)) ([3edae43](https://github.com/ewhauser/shuck/commit/3edae43e35e570659a378fe1f3156dea65efbfd9))
* **linter:** remove test harness context ([#618](https://github.com/ewhauser/shuck/issues/618)) ([ed5d84f](https://github.com/ewhauser/shuck/commit/ed5d84f91948bd19d0ca42461536cf4e64f782f1))
* **linter:** remove unused file context tags ([#617](https://github.com/ewhauser/shuck/issues/617)) ([8e8c803](https://github.com/ewhauser/shuck/commit/8e8c803a1dfbee67959083d520871ad8e29a7491))
* **linter:** use AST for ambient completion contracts ([#624](https://github.com/ewhauser/shuck/issues/624)) ([832c61c](https://github.com/ewhauser/shuck/commit/832c61c02fbf972714e56e62fa43cee986746c28))
* **linter:** use AST operands in safe value ([#628](https://github.com/ewhauser/shuck/issues/628)) ([b4b8923](https://github.com/ewhauser/shuck/commit/b4b89234c5b5b9934a082ca000f2f896d44f3f40))
* **linter:** use semantic declaration operands ([#632](https://github.com/ewhauser/shuck/issues/632)) ([a7cf8ce](https://github.com/ewhauser/shuck/commit/a7cf8ce3623ebc4ceb5d1bfd889a1bc9c24f252f))
* **semantic:** collect file-entry contracts during traversal ([#626](https://github.com/ewhauser/shuck/issues/626)) ([462ee56](https://github.com/ewhauser/shuck/commit/462ee5614005c1d60b024bf2584457ebaa179267))

## [0.0.23](https://github.com/ewhauser/shuck/compare/v0.0.22...v0.0.23) (2026-04-26)


### Features

* **cli:** support per-file shell overrides ([#608](https://github.com/ewhauser/shuck/issues/608)) ([fbfe3e2](https://github.com/ewhauser/shuck/commit/fbfe3e2f91932b4927e26cd1ab04cdf444784e5c))


### Bug Fixes

* **linter:** align S001 indirect expansion parity ([#596](https://github.com/ewhauser/shuck/issues/596)) ([6341fc9](https://github.com/ewhauser/shuck/commit/6341fc952b7badbf400b99fcf6f8b732af221d92))
* **linter:** align S001 safe optional values ([#594](https://github.com/ewhauser/shuck/issues/594)) ([7575800](https://github.com/ewhauser/shuck/commit/75758001e4e811c09f3488e5b7fc4679ee4f1a07))
* **linter:** clear S001 initializer self-reference divergences ([#598](https://github.com/ewhauser/shuck/issues/598)) ([2f7b5d7](https://github.com/ewhauser/shuck/commit/2f7b5d75bfb54e1b81ed857b20ef78d451392892))
* **linter:** generalize C006 build-flag parity ([#580](https://github.com/ewhauser/shuck/issues/580)) ([ff1c1dc](https://github.com/ewhauser/shuck/commit/ff1c1dcbc08fa83a2209b47e7acbd3a0a1aa12c5))
* **linter:** generalize xargs inline replace parity ([#581](https://github.com/ewhauser/shuck/issues/581)) ([6ac42f2](https://github.com/ewhauser/shuck/commit/6ac42f2cd684cdbcb7bd14c3720c0d152b68eaa8))
* **linter:** move C005 exemptions into facts ([#583](https://github.com/ewhauser/shuck/issues/583)) ([77b6028](https://github.com/ewhauser/shuck/commit/77b60286dd347fbea9b1eb8143309b30098ad859))
* **linter:** share shell dialect parsing policy ([#600](https://github.com/ewhauser/shuck/issues/600)) ([925b6c4](https://github.com/ewhauser/shuck/commit/925b6c49fe8d844e983e4c749c6d88b6cf63cd78))
* **linter:** stop ambient contracts initializing runtime names ([#582](https://github.com/ewhauser/shuck/issues/582)) ([4ec1349](https://github.com/ewhauser/shuck/commit/4ec1349260d0f4020c512e978973562de4eeb6cd))
* **website:** keep rule docs in sync ([#607](https://github.com/ewhauser/shuck/issues/607)) ([c687090](https://github.com/ewhauser/shuck/commit/c687090269637e1a1cffb1bfe63951af7e20aaaf))


### Performance

* **linter:** avoid sorting command fact relationships ([#590](https://github.com/ewhauser/shuck/issues/590)) ([789d785](https://github.com/ewhauser/shuck/commit/789d785b9269ca9fa00c256a4fb34551ad9a808f))
* **linter:** cache C133 builtin array history ([#602](https://github.com/ewhauser/shuck/issues/602)) ([697b6b1](https://github.com/ewhauser/shuck/commit/697b6b16b4f330182b443f4af29d6111d59515ab))
* **linter:** index C063 activation windows ([#597](https://github.com/ewhauser/shuck/issues/597)) ([8954525](https://github.com/ewhauser/shuck/commit/8954525d7ba5adfa9b38e3196749c8a95a9fb346))
* **linter:** index possible misspelling candidates ([#603](https://github.com/ewhauser/shuck/issues/603)) ([1e6ba31](https://github.com/ewhauser/shuck/commit/1e6ba31f9f3c64e887b13c307ac714cbddb3df86))
* **linter:** reduce facts allocation churn ([#584](https://github.com/ewhauser/shuck/issues/584)) ([b7cbee1](https://github.com/ewhauser/shuck/commit/b7cbee11bffb65044638618d712b50717794e468))
* **linter:** reduce facts-layer allocation churn ([#578](https://github.com/ewhauser/shuck/issues/578)) ([f0a282f](https://github.com/ewhauser/shuck/commit/f0a282fc9d6c47c1a3fcc5e3dc8ed745f1e3e322))
* **linter:** reuse command relationships in facts ([#592](https://github.com/ewhauser/shuck/issues/592)) ([585bbe9](https://github.com/ewhauser/shuck/commit/585bbe964170f5c780113c6482cb6d26dbde2b92))
* **linter:** reuse command relationships in more facts ([#593](https://github.com/ewhauser/shuck/issues/593)) ([e0b8708](https://github.com/ewhauser/shuck/commit/e0b8708728e13ef7f52922a529b08c7029ee38ed))
* **semantic:** avoid eager reaching map materialization ([#591](https://github.com/ewhauser/shuck/issues/591)) ([22fe078](https://github.com/ewhauser/shuck/commit/22fe078d8552d159df8c966f04fae969107f3fc6))
* **semantic:** index parameter guard flow refs ([#605](https://github.com/ewhauser/shuck/issues/605)) ([d6bb74d](https://github.com/ewhauser/shuck/commit/d6bb74d2d3523b939ab8a1002d230ec264183799))
* **semantic:** reduce CFG allocation churn ([#587](https://github.com/ewhauser/shuck/issues/587)) ([1e66db0](https://github.com/ewhauser/shuck/commit/1e66db092dd7ea456746aca451a526037cfd05f7))
* speed up large corpus hotspot analysis ([#585](https://github.com/ewhauser/shuck/issues/585)) ([1fc11ae](https://github.com/ewhauser/shuck/commit/1fc11aefbae0147bb818cc85ff03bf0ce9d155a4))


### Documentation

* add AST arena migration spec ([#586](https://github.com/ewhauser/shuck/issues/586)) ([07567bb](https://github.com/ewhauser/shuck/commit/07567bb7d66e9cf18ac73af1adb184cefc9f93eb))
* add suppression guide ([#611](https://github.com/ewhauser/shuck/issues/611)) ([239f4c8](https://github.com/ewhauser/shuck/commit/239f4c87ff7b2736e9ace162e033279d7c9f0f5d))
* **website:** generate settings reference ([#610](https://github.com/ewhauser/shuck/issues/610)) ([fc381df](https://github.com/ewhauser/shuck/commit/fc381df6a2c872752cb5d0e1ca9b6b3273a46003))


### Refactor

* **formatter:** replace formatter implementation with stubs ([#588](https://github.com/ewhauser/shuck/issues/588)) ([800d9f3](https://github.com/ewhauser/shuck/commit/800d9f3a089a0e8f19a3741abf866ff5e6998c08))

## [0.0.22](https://github.com/ewhauser/shuck/compare/v0.0.21...v0.0.22) (2026-04-25)


### Bug Fixes

* **cache:** tighten file cache invalidation ([#548](https://github.com/ewhauser/shuck/issues/548)) ([ea222d7](https://github.com/ewhauser/shuck/commit/ea222d7ce37c20f2db24167afd6968a8dcdeb7d4))
* **cli:** preserve parse failure exit status ([#549](https://github.com/ewhauser/shuck/issues/549)) ([6281202](https://github.com/ewhauser/shuck/commit/628120254cae74f4eca3f57b71dec47be1c31ff8))
* **linter:** align C001 conformance ([#556](https://github.com/ewhauser/shuck/issues/556)) ([92a1d98](https://github.com/ewhauser/shuck/commit/92a1d9861951eee0494cdf57a72c79d4a67ece55))
* **linter:** align C057 with SC2328 ([#567](https://github.com/ewhauser/shuck/issues/567)) ([6034796](https://github.com/ewhauser/shuck/commit/6034796858dab26d1fefe5dd8af54c686189dd28))
* **linter:** align C124 corpus behavior ([#551](https://github.com/ewhauser/shuck/issues/551)) ([0180c6e](https://github.com/ewhauser/shuck/commit/0180c6e4d5a785501b42779d9db8680d45fcc606))
* **linter:** align compat source closure policy ([#552](https://github.com/ewhauser/shuck/issues/552)) ([43975b9](https://github.com/ewhauser/shuck/commit/43975b9f740d5b7304a3aae7fabe50c622d17e6d))
* **linter:** clear C063 corpus divergences ([#577](https://github.com/ewhauser/shuck/issues/577)) ([e776812](https://github.com/ewhauser/shuck/commit/e776812170a7a0c2ad5d34eda45fdc823423b4f3))
* **linter:** eliminate C006 corpus divergences ([#574](https://github.com/ewhauser/shuck/issues/574)) ([0511db8](https://github.com/ewhauser/shuck/commit/0511db8d17858bf2dc21c7a34c2a509e534cec3d))
* **linter:** generalize C156 reference candidates ([#569](https://github.com/ewhauser/shuck/issues/569)) ([1ca7fad](https://github.com/ewhauser/shuck/commit/1ca7fadd9cbf97c51da2943a8311f04955a29b2a))
* **linter:** improve C063 ShellCheck compatibility ([#570](https://github.com/ewhauser/shuck/issues/570)) ([a289b46](https://github.com/ewhauser/shuck/commit/a289b46a1207021f5da7399ecf4628b20b6cc165))
* **linter:** improve S001 ShellCheck parity ([#576](https://github.com/ewhauser/shuck/issues/576)) ([16e766e](https://github.com/ewhauser/shuck/commit/16e766e0fc2b038411a053d1a649dbd1077ce8c6))
* **linter:** match xargs zero-option parity ([#572](https://github.com/ewhauser/shuck/issues/572)) ([4e2545e](https://github.com/ewhauser/shuck/commit/4e2545eb8ef13421cc50e3488a64a44f072e4699))
* **linter:** preserve C006 reports after subscript reads ([#555](https://github.com/ewhauser/shuck/issues/555)) ([a6310f6](https://github.com/ewhauser/shuck/commit/a6310f6bca0683cc114a991dd859b74e2c140682))
* **linter:** reduce C124 corpus divergences ([#544](https://github.com/ewhauser/shuck/issues/544)) ([b306cf5](https://github.com/ewhauser/shuck/commit/b306cf56632de70efeeef8cfd0be7432b2b54ce7))
* **linter:** reduce S001 false positives ([#547](https://github.com/ewhauser/shuck/issues/547)) ([139a884](https://github.com/ewhauser/shuck/commit/139a884821ab8f20edaf772c2f9f85c32a329831))
* **linter:** remove project-specific ambient contracts ([#571](https://github.com/ewhauser/shuck/issues/571)) ([c0858a6](https://github.com/ewhauser/shuck/commit/c0858a6be375fb7b1fa91c37d178924b1ca0f71c))
* **linter:** report C006 indexed subscript keys ([#553](https://github.com/ewhauser/shuck/issues/553)) ([d6e72f2](https://github.com/ewhauser/shuck/commit/d6e72f2793989510e5b07b305c5911fbfbce750a))
* **linter:** report declaration-only C001 targets ([#546](https://github.com/ewhauser/shuck/issues/546)) ([13bc56e](https://github.com/ewhauser/shuck/commit/13bc56e7d34fce709c6f237541255220844c9346))
* **linter:** report S004 in command wrapper targets ([#545](https://github.com/ewhauser/shuck/issues/545)) ([a29d911](https://github.com/ewhauser/shuck/commit/a29d911bf5d822770350804d575c6be26db59534))


### Performance

* **linter:** finish indexed fact arenas ([#575](https://github.com/ewhauser/shuck/issues/575)) ([9d0b0c1](https://github.com/ewhauser/shuck/commit/9d0b0c1c6debeaa8c0e1c93d7c0dd71e85bbe139))
* **linter:** pack facts into indexed arenas ([#538](https://github.com/ewhauser/shuck/issues/538)) ([e1d7f27](https://github.com/ewhauser/shuck/commit/e1d7f27ce2af599dd2506ce74b24a05f96f05ac5))
* **linter:** reduce fact traversal overhead ([#557](https://github.com/ewhauser/shuck/issues/557)) ([4ce42ec](https://github.com/ewhauser/shuck/commit/4ce42ecf027bd61456888077c9fe625290056c00))
* **linter:** reduce scratch allocation churn ([#564](https://github.com/ewhauser/shuck/issues/564)) ([c33046c](https://github.com/ewhauser/shuck/commit/c33046c33f3716dc139333339488920d08ed628c))
* **linter:** reuse analyzed path set ([#550](https://github.com/ewhauser/shuck/issues/550)) ([213ba04](https://github.com/ewhauser/shuck/commit/213ba04a3ddb2291f074c92c4d06086ba155c45f))
* **linter:** trim fact graph allocations ([#560](https://github.com/ewhauser/shuck/issues/560)) ([655d2fe](https://github.com/ewhauser/shuck/commit/655d2fea251d3609a3e43e4e158b78548c5ad2f4))
* **parser:** avoid brace scan allocations ([#561](https://github.com/ewhauser/shuck/issues/561)) ([d7c91e1](https://github.com/ewhauser/shuck/commit/d7c91e1742642fd299cab4f7e29af3a49dd32057))
* **parser:** reduce checkpoint allocations ([#566](https://github.com/ewhauser/shuck/issues/566)) ([ff95f65](https://github.com/ewhauser/shuck/commit/ff95f654f28cf1ac5ecc7996009d435c3f9bee02))
* **parser:** reduce word construction allocations ([#563](https://github.com/ewhauser/shuck/issues/563)) ([42d953d](https://github.com/ewhauser/shuck/commit/42d953d1d2882a904454a3755effd116c258fa6c))
* **parser:** reduce word subscript allocations ([#565](https://github.com/ewhauser/shuck/issues/565)) ([3da8583](https://github.com/ewhauser/shuck/commit/3da85831a9c13e913dbe6947ec06826c0567569c))
* **semantic:** reduce CFG vector allocations ([#562](https://github.com/ewhauser/shuck/issues/562)) ([5b359be](https://github.com/ewhauser/shuck/commit/5b359be6adcd3e3325aa456e8ed585e3c49be3e1))
* **semantic:** reuse dataflow bitset buffers ([#558](https://github.com/ewhauser/shuck/issues/558)) ([1aeb3fd](https://github.com/ewhauser/shuck/commit/1aeb3fd75a2a9861282fb72a92ecff29bef4760d))


### Documentation

* refresh architecture and rule guidance ([#559](https://github.com/ewhauser/shuck/issues/559)) ([53f9b58](https://github.com/ewhauser/shuck/commit/53f9b58b8e6350ede68c19f5479168d83cc24bf2))
* **website:** add shellcheck repo conformance table ([#554](https://github.com/ewhauser/shuck/issues/554)) ([8831b8a](https://github.com/ewhauser/shuck/commit/8831b8a25da8f5cee4f27abefd8399d49036c0c1))

## [0.0.21](https://github.com/ewhauser/shuck/compare/v0.0.20...v0.0.21) (2026-04-24)


### Bug Fixes

* **cli:** include analyzed paths in check cache key ([#532](https://github.com/ewhauser/shuck/issues/532)) ([3a010c8](https://github.com/ewhauser/shuck/commit/3a010c8db2859939b1122b85b4adc62421a35c58))
* **linter:** add C006 parameter guard flow ([#539](https://github.com/ewhauser/shuck/issues/539)) ([9df2595](https://github.com/ewhauser/shuck/commit/9df25953d35eeeee0491123343c40a194343d169))
* **linter:** align C001 with ShellCheck corpus ([#501](https://github.com/ewhauser/shuck/issues/501)) ([b37f85b](https://github.com/ewhauser/shuck/commit/b37f85b26a51a5b14abb8cf38c07d625d40df7cb))
* **linter:** align C124 unreachable causes ([#533](https://github.com/ewhauser/shuck/issues/533)) ([0bc715e](https://github.com/ewhauser/shuck/commit/0bc715eda800435ff11cd684c5868caf6afd8457))
* **linter:** broaden ambient runtime contracts ([#541](https://github.com/ewhauser/shuck/issues/541)) ([01aabd0](https://github.com/ewhauser/shuck/commit/01aabd003c87bd6aa4d37021c62529b107300c79))
* **linter:** broaden C063 function reachability ([#537](https://github.com/ewhauser/shuck/issues/537)) ([3b6bde9](https://github.com/ewhauser/shuck/commit/3b6bde9c57e2d0909b4b88c0d9c997c858d33ea0))
* **linter:** improve S001 ShellCheck parity ([#521](https://github.com/ewhauser/shuck/issues/521)) ([24ad0a6](https://github.com/ewhauser/shuck/commit/24ad0a6067d9419a74f8cb6c6c3f1420c0e5c714))
* **linter:** match C063 nested function reachability ([#542](https://github.com/ewhauser/shuck/issues/542)) ([7f99c9d](https://github.com/ewhauser/shuck/commit/7f99c9df12ac2b239c9373fd10ca974cd19c1289))
* **linter:** skip C124 short-circuit exit guards ([#540](https://github.com/ewhauser/shuck/issues/540)) ([8fed78b](https://github.com/ewhauser/shuck/commit/8fed78b15bad71a6210f9e3d4f4efbb83adf9cef))
* recognize env -S shebangs ([#534](https://github.com/ewhauser/shuck/issues/534)) ([81438ac](https://github.com/ewhauser/shuck/commit/81438ac647c778675117f3d8b9472baf18af39ab))
* **semantic:** infer sourced helper parse profiles ([#535](https://github.com/ewhauser/shuck/issues/535)) ([5e912b6](https://github.com/ewhauser/shuck/commit/5e912b6161bc3e30c044ca5072922d3bc1cf8e1c))

## [0.0.20](https://github.com/ewhauser/shuck/compare/v0.0.19...v0.0.20) (2026-04-24)


### Bug Fixes

* **linter:** align C087 with SC2072 ([#526](https://github.com/ewhauser/shuck/issues/526)) ([6ad0b33](https://github.com/ewhauser/shuck/commit/6ad0b33a466c84a95d9ea1bfa560f2921c976e82))
* **linter:** align C091 with ShellCheck oracle ([#527](https://github.com/ewhauser/shuck/issues/527)) ([40eeb03](https://github.com/ewhauser/shuck/commit/40eeb038b93dec25d7e168efaeb51911c0064227))
* **linter:** align C123 with shellcheck ([#528](https://github.com/ewhauser/shuck/issues/528)) ([301f548](https://github.com/ewhauser/shuck/commit/301f548397d1d34a5ef31f7ce7ab8a2c714a85f1))
* **linter:** align C125 with ShellCheck ([#519](https://github.com/ewhauser/shuck/issues/519)) ([d5cfbec](https://github.com/ewhauser/shuck/commit/d5cfbecb387cf9fb2d414909cfa2afb66718fb0f))
* **linter:** align C133 with ShellCheck ([#514](https://github.com/ewhauser/shuck/issues/514)) ([df83710](https://github.com/ewhauser/shuck/commit/df83710ea8ade3b8f20eded17be73aa376fbced4))
* **linter:** align C156 with ShellCheck oracle ([#509](https://github.com/ewhauser/shuck/issues/509)) ([f72f33f](https://github.com/ewhauser/shuck/commit/f72f33fb71bf65eab2da8024b60b9c4b91297d5d))
* **linter:** align S016 echo substitution checks ([#518](https://github.com/ewhauser/shuck/issues/518)) ([f3fb209](https://github.com/ewhauser/shuck/commit/f3fb209b0795a95fb5dc1afad3ad2ccb45c3b712))
* **linter:** align S017 brace fanout behavior ([#516](https://github.com/ewhauser/shuck/issues/516)) ([544db2c](https://github.com/ewhauser/shuck/commit/544db2c278ae8f62347858627c6ab70507360988))
* **linter:** align S045 with shellcheck ([#512](https://github.com/ewhauser/shuck/issues/512)) ([7527115](https://github.com/ewhauser/shuck/commit/7527115604c2c4e13d7d6e87f840546ea39cc269))
* **linter:** align S057 with alias parameter oracle ([#520](https://github.com/ewhauser/shuck/issues/520)) ([2c2bb08](https://github.com/ewhauser/shuck/commit/2c2bb0885fd8a4210af94528dad4d4ceedb45a3e))
* **linter:** align S067 with ShellCheck ([#515](https://github.com/ewhauser/shuck/issues/515)) ([1ae864e](https://github.com/ewhauser/shuck/commit/1ae864e5624536dfb10c3a1e254247d4c5ddd53f))
* **linter:** align S070 with ShellCheck oracle ([#529](https://github.com/ewhauser/shuck/issues/529)) ([4ff3e76](https://github.com/ewhauser/shuck/commit/4ff3e7699032761082334e762c3cf7ef7b222a42))
* **linter:** align X035 named coproc parity ([#517](https://github.com/ewhauser/shuck/issues/517)) ([a1cc9c3](https://github.com/ewhauser/shuck/commit/a1cc9c30cfd43b6c09590e3edfab4d56ee10b2e7))


### Performance

* **parser:** retain compact AST command containers ([#525](https://github.com/ewhauser/shuck/issues/525)) ([d896a8c](https://github.com/ewhauser/shuck/commit/d896a8c12c06c336165e408bd17e2b535efb10b3))
* **parser:** stop over-reserving compound lists ([#524](https://github.com/ewhauser/shuck/issues/524)) ([c9ed99c](https://github.com/ewhauser/shuck/commit/c9ed99cd97ea46fe6404c868b6e84ba50b4750a0))


### Refactor

* **linter:** rename parse-result lint entrypoint ([#522](https://github.com/ewhauser/shuck/issues/522)) ([9088809](https://github.com/ewhauser/shuck/commit/908880943d766cda68889a5ae90c6ee6fba11b0d))

## [0.0.19](https://github.com/ewhauser/shuck/compare/v0.0.18...v0.0.19) (2026-04-23)


### Bug Fixes

* **linter:** align C061 command-name conformance ([#442](https://github.com/ewhauser/shuck/issues/442)) ([a342790](https://github.com/ewhauser/shuck/commit/a3427901ddd2f5fd433007d6db1146837711735c))
* **linter:** align C077 with ShellCheck oracle ([#478](https://github.com/ewhauser/shuck/issues/478)) ([f6fb010](https://github.com/ewhauser/shuck/commit/f6fb01051843c0faf9499b9bf085026b2ccd7dcd))
* **linter:** align C092 with shellcheck ([#499](https://github.com/ewhauser/shuck/issues/499)) ([efc90e2](https://github.com/ewhauser/shuck/commit/efc90e201f532cdafe7525a508eb39b8c9922b8e))
* **linter:** align C094 with ShellCheck ([#470](https://github.com/ewhauser/shuck/issues/470)) ([75c41fc](https://github.com/ewhauser/shuck/commit/75c41fcdf123413d103505c7fea471a064848a48))
* **linter:** align C094 with ShellCheck oracle ([#484](https://github.com/ewhauser/shuck/issues/484)) ([ac996a6](https://github.com/ewhauser/shuck/commit/ac996a6e5003258de2a4c5caf577f84d087942bf))
* **linter:** align C095 with ShellCheck ([#474](https://github.com/ewhauser/shuck/issues/474)) ([126e55e](https://github.com/ewhauser/shuck/commit/126e55e1ded20e2c30531e8941902f6bbf317497))
* **linter:** align C105 with shellcheck ([#495](https://github.com/ewhauser/shuck/issues/495)) ([9732a6b](https://github.com/ewhauser/shuck/commit/9732a6b77d076dfd4d42437480ad6bdb395a1d20))
* **linter:** align C121 variable-name suppression with shellcheck ([#451](https://github.com/ewhauser/shuck/issues/451)) ([6f1e384](https://github.com/ewhauser/shuck/commit/6f1e384a56f27a9353ac108ead24cadb3837ba5a))
* **linter:** align C124 unreachable parity ([#463](https://github.com/ewhauser/shuck/issues/463)) ([0e0c4e2](https://github.com/ewhauser/shuck/commit/0e0c4e27f4c4e371030d9ad21f1650789f0e3258))
* **linter:** align C133 with shellcheck rebinding semantics ([#489](https://github.com/ewhauser/shuck/issues/489)) ([7d9d97f](https://github.com/ewhauser/shuck/commit/7d9d97fe193c92c33edf838aec99fa6dc3498703))
* **linter:** align C150 loop spans with ShellCheck ([#507](https://github.com/ewhauser/shuck/issues/507)) ([f0c662a](https://github.com/ewhauser/shuck/commit/f0c662ad4032c3e2335d8abcbc34324a6546d8e3))
* **linter:** align C155 subshell side effects ([#485](https://github.com/ewhauser/shuck/issues/485)) ([b7520fa](https://github.com/ewhauser/shuck/commit/b7520fa32c46852b6678b85b147f46ca9835ca9a))
* **linter:** align C156 with oracle ([#482](https://github.com/ewhauser/shuck/issues/482)) ([17a5f90](https://github.com/ewhauser/shuck/commit/17a5f90d8321573889622bafaaa899b23263e47e))
* **linter:** align K001 with ShellCheck behavior ([#496](https://github.com/ewhauser/shuck/issues/496)) ([31cabc6](https://github.com/ewhauser/shuck/commit/31cabc616e419e4126405eba6919ae384021ac16))
* **linter:** align S004 subscript handling with shellcheck ([#450](https://github.com/ewhauser/shuck/issues/450)) ([cbe1840](https://github.com/ewhauser/shuck/commit/cbe184016ab143df5dd0a98b9b9feefa4dde0535))
* **linter:** align S008 with shellcheck oracle ([#475](https://github.com/ewhauser/shuck/issues/475)) ([ba3d4d1](https://github.com/ewhauser/shuck/commit/ba3d4d1309696e20cafaac9ea26b29a4994fb613))
* **linter:** align S015 and restore large-corpus parity ([#494](https://github.com/ewhauser/shuck/issues/494)) ([91f9683](https://github.com/ewhauser/shuck/commit/91f9683e275b3f8796612694b0f23e353f47c975))
* **linter:** align S019 with shellcheck ([#490](https://github.com/ewhauser/shuck/issues/490)) ([0cc3fd9](https://github.com/ewhauser/shuck/commit/0cc3fd9e64bf932c0d314e58a406c5d7717ee8a4))
* **linter:** align S020 with shellcheck ([#459](https://github.com/ewhauser/shuck/issues/459)) ([9a1927d](https://github.com/ewhauser/shuck/commit/9a1927daf0b337c2bcefeed4289317e16f39ccd2))
* **linter:** align S029 escaped template braces ([#471](https://github.com/ewhauser/shuck/issues/471)) ([ee74249](https://github.com/ewhauser/shuck/commit/ee74249c97b9a33a7c19cb1a3d5c783a458466f2))
* **linter:** align S038 with ShellCheck behavior ([#467](https://github.com/ewhauser/shuck/issues/467)) ([6cf6a25](https://github.com/ewhauser/shuck/commit/6cf6a2566021960d40b6b4b7edbe3a4d564a93ec))
* **linter:** align S041 function body checks with ShellCheck ([#481](https://github.com/ewhauser/shuck/issues/481)) ([b0bf89e](https://github.com/ewhauser/shuck/commit/b0bf89e3dc2bf14c7af260e716798929f0ad7c50))
* **linter:** align S044 with shellcheck ([#497](https://github.com/ewhauser/shuck/issues/497)) ([25adc28](https://github.com/ewhauser/shuck/commit/25adc2864244fa74f0041c864815be5710cae5d5))
* **linter:** align S047 with ShellCheck ([#503](https://github.com/ewhauser/shuck/issues/503)) ([1cf1d7f](https://github.com/ewhauser/shuck/commit/1cf1d7f5810705315bfcd088b74769a95240b5e9))
* **linter:** align S054 with shellcheck behavior ([#492](https://github.com/ewhauser/shuck/issues/492)) ([d905ffa](https://github.com/ewhauser/shuck/commit/d905ffaa153c61c227cdf31ed801267a1944aaef))
* **linter:** align S064 with ShellCheck ([#500](https://github.com/ewhauser/shuck/issues/500)) ([369a11b](https://github.com/ewhauser/shuck/commit/369a11b7211ea58c48edce9398dc65203cab85b3))
* **linter:** align S068 trap signal rule with oracle ([#479](https://github.com/ewhauser/shuck/issues/479)) ([956ed86](https://github.com/ewhauser/shuck/commit/956ed86251723f35bd7d6980aee167f7b4ebfbee))
* **linter:** align S076 with ShellCheck ([#476](https://github.com/ewhauser/shuck/issues/476)) ([6adc3b0](https://github.com/ewhauser/shuck/commit/6adc3b0ca7362e26161e1e1595b3c0d566a7c1c6))
* **linter:** align X004 spans with ShellCheck ([#498](https://github.com/ewhauser/shuck/issues/498)) ([005ddde](https://github.com/ewhauser/shuck/commit/005dddeb3899f60c74dc1ebec7d4a13215c0fbf8))
* **linter:** align X005 case fallthrough spans ([#510](https://github.com/ewhauser/shuck/issues/510)) ([0a0e73c](https://github.com/ewhauser/shuck/commit/0a0e73c33aedf21c51d8e1b8ab0c1c7e0f66eca7))
* **linter:** align X010 with ShellCheck ([#488](https://github.com/ewhauser/shuck/issues/488)) ([28145b2](https://github.com/ewhauser/shuck/commit/28145b27a7334c38b793f86aa0308321a9460692))
* **linter:** align X031 source scope with ShellCheck ([#502](https://github.com/ewhauser/shuck/issues/502)) ([f5582ad](https://github.com/ewhauser/shuck/commit/f5582ade098f292612b4e104f6936aed022001bf))
* **linter:** align X040 with shellcheck ([#487](https://github.com/ewhauser/shuck/issues/487)) ([a5e2292](https://github.com/ewhauser/shuck/commit/a5e2292996cec4de385dec71ff2329b8d7f95e21))
* **linter:** align X043 split modifiers with ShellCheck ([#473](https://github.com/ewhauser/shuck/issues/473)) ([b96b493](https://github.com/ewhauser/shuck/commit/b96b493a9294b52b1c020d62507f7184c3604de3))
* **linter:** align X080 with ShellCheck source directives ([#511](https://github.com/ewhauser/shuck/issues/511)) ([ff09fb9](https://github.com/ewhauser/shuck/commit/ff09fb932b685d13e7d8bba29d7263ff39d76cda))
* **linter:** align X081 with shellcheck ([#491](https://github.com/ewhauser/shuck/issues/491)) ([7aea68d](https://github.com/ewhauser/shuck/commit/7aea68da4f6e6c1012989bbd31ce0316945ff568))
* **linter:** avoid repeated scope scans in subshell facts ([#493](https://github.com/ewhauser/shuck/issues/493)) ([947961d](https://github.com/ewhauser/shuck/commit/947961d8054eb03f83cd68ec30e9fb78181fca65))
* **linter:** broaden C099 scalar array assignment detection ([#468](https://github.com/ewhauser/shuck/issues/468)) ([832bf91](https://github.com/ewhauser/shuck/commit/832bf918b62fa321ca229d40038472a891352098))
* **linter:** broaden X016 for non-portable sh builtins ([#458](https://github.com/ewhauser/shuck/issues/458)) ([aa2a4db](https://github.com/ewhauser/shuck/commit/aa2a4db2f2c9101a5e5bf21d22e601aea22fde77))
* **linter:** broaden X021 set -o portability ([#506](https://github.com/ewhauser/shuck/issues/506)) ([c2dca4b](https://github.com/ewhauser/shuck/commit/c2dca4bd39928ceea1dbfd55dc3e2e2d36e2db5b))
* **linter:** broaden X062 arithmetic operator coverage ([#457](https://github.com/ewhauser/shuck/issues/457)) ([8cf4621](https://github.com/ewhauser/shuck/commit/8cf46215f9134a420a6d8b65d6608e21ed79d636))
* **linter:** drop X065 for parameter expansion patterns ([#448](https://github.com/ewhauser/shuck/issues/448)) ([32c8033](https://github.com/ewhauser/shuck/commit/32c803352e984c65cac75d563d50ccccef5d5162))
* **linter:** eliminate S001 shellcheck divergences ([#464](https://github.com/ewhauser/shuck/issues/464)) ([96beb9d](https://github.com/ewhauser/shuck/commit/96beb9dd4cef46d08bef154721c5fdfca30c7850))
* **linter:** expand C100 array reference parity ([#480](https://github.com/ewhauser/shuck/issues/480)) ([c65d98d](https://github.com/ewhauser/shuck/commit/c65d98db94dc2af61bad37cdc5c899e669684e0e))
* **linter:** ignore continued echo spacing in S037 ([#449](https://github.com/ewhauser/shuck/issues/449)) ([9ce3a9c](https://github.com/ewhauser/shuck/commit/9ce3a9cbbe4a72061ea78abdf48da1d9b6ae47c1))
* **linter:** match ShellCheck S021 array splat handling ([#504](https://github.com/ewhauser/shuck/issues/504)) ([41c5c78](https://github.com/ewhauser/shuck/commit/41c5c78d108cfaeb017cd231194772d9ce08b9b3))
* **linter:** restore C124 parity without perf regression ([#508](https://github.com/ewhauser/shuck/issues/508)) ([c21ceb4](https://github.com/ewhauser/shuck/commit/c21ceb4219ff8f6f35a3564458a6d200432c4e8b))
* **linter:** skip X007 in regex operands ([#447](https://github.com/ewhauser/shuck/issues/447)) ([4f010ea](https://github.com/ewhauser/shuck/commit/4f010eaf68ebec62643bd929828bd04650d5c78d))
* **linter:** stop misclassifying arithmetic trims as X070 ([#455](https://github.com/ewhauser/shuck/issues/455)) ([d7cc148](https://github.com/ewhauser/shuck/commit/d7cc1486b029faad04e5510c6052bc1a62b9d17b))
* **linter:** tighten S001 nested substitution parity ([#439](https://github.com/ewhauser/shuck/issues/439)) ([71d588a](https://github.com/ewhauser/shuck/commit/71d588a61bae4300b46c7bb96ae5a14439feb86a))
* **report:** show metadata skips in large corpus HTML report ([#444](https://github.com/ewhauser/shuck/issues/444)) ([ed20544](https://github.com/ewhauser/shuck/commit/ed20544eb2b6e8f981d0d87ad787a72497b4cdd2))


### Performance

* **ast:** speed up static command name decoding ([#486](https://github.com/ewhauser/shuck/issues/486)) ([269ee84](https://github.com/ewhauser/shuck/commit/269ee84105f8daf96ab76b6ca6fa7ffea7cda2b1))


### Reverts

* **linter:** restore C124 macro benchmark performance ([#505](https://github.com/ewhauser/shuck/issues/505)) ([1e34593](https://github.com/ewhauser/shuck/commit/1e34593e515b06e34adcda2abaa73353adb953aa))


### Refactor

* **ast:** centralize static word text helper ([#462](https://github.com/ewhauser/shuck/issues/462)) ([61df36a](https://github.com/ewhauser/shuck/commit/61df36a023d9548389d1e76df220467ac93954b8))
* **linter:** clarify C087 dotted version policy ([#452](https://github.com/ewhauser/shuck/issues/452)) ([44ce6ac](https://github.com/ewhauser/shuck/commit/44ce6ac501be87a76039865eda75aa06ca626e7d))
* **linter:** move command normalization into facts ([#477](https://github.com/ewhauser/shuck/issues/477)) ([701c9eb](https://github.com/ewhauser/shuck/commit/701c9eb6d5c718372cc27781875eae357b013830))
* **linter:** move expansion analysis into facts ([#472](https://github.com/ewhauser/shuck/issues/472)) ([02c0ab4](https://github.com/ewhauser/shuck/commit/02c0ab488a85fb6d01e2cc7f57a73dde182facc9))
* **linter:** move traversal helpers into facts ([#483](https://github.com/ewhauser/shuck/issues/483)) ([85c9125](https://github.com/ewhauser/shuck/commit/85c91256566e1fdbdf18b838e5ee53107666fcd6))
* **linter:** move word classification into facts ([#469](https://github.com/ewhauser/shuck/issues/469)) ([8acf103](https://github.com/ewhauser/shuck/commit/8acf103940c0e665a7ea4cd13e24ab0fd84b40ef))
* **linter:** remove C110 ([#453](https://github.com/ewhauser/shuck/issues/453)) ([46c7c2b](https://github.com/ewhauser/shuck/commit/46c7c2b65b01fb7b2a3695cbabe91cb7c93217b6))
* **linter:** remove S063 ([#466](https://github.com/ewhauser/shuck/issues/466)) ([222f325](https://github.com/ewhauser/shuck/commit/222f3251826fec1ceb747459d1e9d87b62ec30ea))
* **linter:** split span helpers by owner ([#454](https://github.com/ewhauser/shuck/issues/454)) ([3d10446](https://github.com/ewhauser/shuck/commit/3d10446c59cdd34b7fc8134e9ba0d35c771a2f64))
* **linter:** split word helpers by layer ([#465](https://github.com/ewhauser/shuck/issues/465)) ([4b8371a](https://github.com/ewhauser/shuck/commit/4b8371ae5f9b4f6b6f8dd43a37e221d1544d5894))

## [0.0.18](https://github.com/ewhauser/shuck/compare/v0.0.17...v0.0.18) (2026-04-22)


### Features

* lint embedded GitHub Actions scripts ([#417](https://github.com/ewhauser/shuck/issues/417)) ([9182810](https://github.com/ewhauser/shuck/commit/9182810671ade2f001840bea907fb9a2c16b8072))


### Bug Fixes

* **c001:** eliminate shellcheck-only corpus divergences ([#429](https://github.com/ewhauser/shuck/issues/429)) ([476ae8f](https://github.com/ewhauser/shuck/commit/476ae8fabbc121306cc19cff68ade4c9fb4ff44a))
* **c001:** preserve array-like indirect targets in shellcheck compat mode ([#426](https://github.com/ewhauser/shuck/issues/426)) ([b04795b](https://github.com/ewhauser/shuck/commit/b04795bd9ec2ed7d86b8b33ee7257caabe624118))
* **linter:** add autofixes for C084, C085, C086, and C088 ([#432](https://github.com/ewhauser/shuck/issues/432)) ([9a11dd1](https://github.com/ewhauser/shuck/commit/9a11dd117e0b471bc63398152ea0ad82d5f14fc4))
* **linter:** add autofixes for X069, X055, and S023 ([#424](https://github.com/ewhauser/shuck/issues/424)) ([c1c6717](https://github.com/ewhauser/shuck/commit/c1c6717ff6d3c1e077b562c59898cdb9964a1e7c))
* **linter:** align corpus conformance through C012 ([#431](https://github.com/ewhauser/shuck/issues/431)) ([4453834](https://github.com/ewhauser/shuck/commit/445383446b510daffd3a5e018fe31d36aebb6509))
* **linter:** reduce S001 shellcheck divergences ([#430](https://github.com/ewhauser/shuck/issues/430)) ([b37259c](https://github.com/ewhauser/shuck/commit/b37259ccb25dc0c65023f973d613e2d36326cbe4))
* **linter:** remove corpus metadata and align conformance for nine rules ([#438](https://github.com/ewhauser/shuck/issues/438)) ([e4dfcb0](https://github.com/ewhauser/shuck/commit/e4dfcb0aa9c52631d0d408a91c9bef9193b954a5))


### Documentation

* **website:** add rules_lint compatibility guide ([#435](https://github.com/ewhauser/shuck/issues/435)) ([7d77dbe](https://github.com/ewhauser/shuck/commit/7d77dbeb3dcc55584dc1e9437f9370d045831e1e))
* **website:** use executable label in rules_lint example ([#436](https://github.com/ewhauser/shuck/issues/436)) ([3c7e92e](https://github.com/ewhauser/shuck/commit/3c7e92e81350ff071c5a32324d25af671f2c20a5))

## [0.0.17](https://github.com/ewhauser/shuck/compare/v0.0.16...v0.0.17) (2026-04-22)


### Bug Fixes

* **c001:** compat-gate indirect expansion targets ([#422](https://github.com/ewhauser/shuck/issues/422)) ([2cdc137](https://github.com/ewhauser/shuck/commit/2cdc137f1734a2bb24757c121e2b8a026a7cd027))
* **cli:** align default rule baseline with shellcheck compat ([#421](https://github.com/ewhauser/shuck/issues/421)) ([54b14dd](https://github.com/ewhauser/shuck/commit/54b14dd53a968a67c698239c46215c9f98eafd68))
* **cli:** use shellcheck metadata levels in compat mode ([#423](https://github.com/ewhauser/shuck/issues/423)) ([cb4f8d8](https://github.com/ewhauser/shuck/commit/cb4f8d8d9689a2896c7101eef084ecfd1496e4d7))
* **compat:** populate ShellCheck rule levels ([#420](https://github.com/ewhauser/shuck/issues/420)) ([be7810f](https://github.com/ewhauser/shuck/commit/be7810f1c1b398a5cd4e5ccda4c5451f5762bad9))
* **linter:** add autofixes for completed backlog rules ([#419](https://github.com/ewhauser/shuck/issues/419)) ([906d74b](https://github.com/ewhauser/shuck/commit/906d74bb7f19405338c535fc274da8a7d231fb98))

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
