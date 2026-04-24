# Changelog

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
