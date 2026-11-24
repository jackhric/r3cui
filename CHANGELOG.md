# Changelog

## [0.2.0](https://github.com/jackhric/r3cui/compare/v0.1.0...v0.2.0) (2026-11-24)


### Features

* **backlight:** add per-zone colour frame engine ([95c44f8](https://github.com/jackhric/r3cui/commit/95c44f8600a3079663471e2f322abacadbb7c27f))
* **backlight:** add soft-white underlay, per-key lights and HA presets ([e831aea](https://github.com/jackhric/r3cui/commit/e831aea675660e3a4b9a6fce51fa40b7882bb745))
* **backlight:** follow the screen; voice key and status line open inputs ([ef319f7](https://github.com/jackhric/r3cui/commit/ef319f7dfa81c34c7e472b4e96925556fee78ca3))
* **backlight:** light keypad in one colour, power zone alone when needed ([278236c](https://github.com/jackhric/r3cui/commit/278236c3e2e89732ea5d8857110bd79de1bdc173))
* **backlight:** steer key backlight from Home Assistant ([e1372ee](https://github.com/jackhric/r3cui/commit/e1372eee0a49242e4ba4ce974fdcbb2b61c446d0))
* **config:** add settings file with hidden receiver inputs ([370aecc](https://github.com/jackhric/r3cui/commit/370aecc3cdfd2de8616e6afae12fdcd83a2faa87))
* **ha:** add Home Assistant theater package with the receiver's input list ([5a933f1](https://github.com/jackhric/r3cui/commit/5a933f1ad0ae279c45992e15b094e8518440f470))
* **haptics:** add feedback engine with haptic patterns, cue sounds and lab ([ca3b50e](https://github.com/jackhric/r3cui/commit/ca3b50e8ce006922f16e826fea558d60695151a6))
* **haptics:** limit haptics to OK, new pages and volume; one hold overlay ([496c504](https://github.com/jackhric/r3cui/commit/496c5044a37a0a676a179fbff375ae980b42ed33))
* **haptics:** play real effects through UC_HAPTIC_DEV_PATH ([da11ee9](https://github.com/jackhric/r3cui/commit/da11ee916f754292d9010bd6521b71d5dfaec41d))
* **haptics:** split Feedback lab into Patterns and Raw effects screens ([79510a1](https://github.com/jackhric/r3cui/commit/79510a12c3c96d08fd877d24b3d593c98a20020b))
* **library:** add library layout prototypes recorded from Kodi ([d7e6cff](https://github.com/jackhric/r3cui/commit/d7e6cffa9f5ecd7c1ecf231b0f05bb67a5398d6c))
* **library:** add live poster wall mirroring Kodi's Movies and TV lists ([000ecad](https://github.com/jackhric/r3cui/commit/000ecad49974afe14c0b3f235516ce8ac0ea26d8))
* **library:** draw MPA and TV certification badges as SVGs ([27318c4](https://github.com/jackhric/r3cui/commit/27318c43fe0bb9c2787fb093287433f63ec54d6a))
* **library:** draw rating stars and stagger poster wall tiles ([4da4045](https://github.com/jackhric/r3cui/commit/4da404598706526c0fc950a868106cb101624730))
* **library:** fixed details layout with a four-line plot box ([82adf7e](https://github.com/jackhric/r3cui/commit/82adf7ed0fb1fa5eb13b4a3205eb21c80bfe1f96))
* **library:** show the highlighted title's plot in three wall styles ([c480850](https://github.com/jackhric/r3cui/commit/c48085080546fc3f8a9d6077437540c19b3fbed9))
* **now-playing:** add backdrop, skips, track pickers and slider volume ([a6e832d](https://github.com/jackhric/r3cui/commit/a6e832dc1a9a6f6f76371015829e2253b3fdd19a))
* **now-playing:** take accent from the poster's most vibrant colour ([fb9195e](https://github.com/jackhric/r3cui/commit/fb9195e13d7a8bc82e1f0e99aed4aa20abaad4c6))
* **ui:** 6 px poster corners; demo can show any film ([2a1ea0d](https://github.com/jackhric/r3cui/commit/2a1ea0d67eb5a7c152736e5c8e2086d3f62464ae))
* **ui:** add a feature recording script that settles the poster wall on one title ([feb00c5](https://github.com/jackhric/r3cui/commit/feb00c5c26f9061a2777bf6bab607dc758d4e032))
* **ui:** add stage, options and overlay layers with codec logos ([1154ff0](https://github.com/jackhric/r3cui/commit/1154ff0acd9f6dc0e9e4e0bd6ed78207deb576b8))
* **ui:** add theater remote UI with live core data, power and sleep handling ([f534ac7](https://github.com/jackhric/r3cui/commit/f534ac7eca216d3daf7e428a17b331e8e35a5571))
* **ui:** add Wi-Fi setup, d-pad keyboard, status page and home warnings ([1f29179](https://github.com/jackhric/r3cui/commit/1f2917971061ddca0a41b497e751498f58eae31d))
* **ui:** blur-through transitions between main stages ([491359f](https://github.com/jackhric/r3cui/commit/491359fdd1801f85167511285459d226dbcdeed7))
* **ui:** enlarge status bar and Kodi mark; hold Home for input list ([cb08573](https://github.com/jackhric/r3cui/commit/cb08573387fb289b1ecd8ef1c6a1c4674f58216c))
* **ui:** follow the receiver input for interface colour ([d608f60](https://github.com/jackhric/r3cui/commit/d608f60ae832fce35a3e773162f697d2d3db8991))
* **ui:** one fixed navbar above every layer ([bdedd21](https://github.com/jackhric/r3cui/commit/bdedd2106da48c8d54666a4e5165881966912cd0))
* **ui:** preview switches for screenshots against the simulator ([1e2722d](https://github.com/jackhric/r3cui/commit/1e2722df9e8269676912bde9fcf5eeb1746ac9ab))
* **ui:** scroll option lists with focus tracking and a position bar ([d745bfe](https://github.com/jackhric/r3cui/commit/d745bfef15702df0908bf2ec8347a1021bd7df58))
* **ui:** send brightness to the remote; lock slider under auto brightness ([65c5570](https://github.com/jackhric/r3cui/commit/65c55700dcb92496195afdf147e15c875a1c8a4c))
* **ui:** show a picked input immediately with a sheen until confirmed ([fe9eef6](https://github.com/jackhric/r3cui/commit/fe9eef6850258d85645776b1f6ada5c7a26a0f95))
* **ui:** show every input's mark on the home screen ([dd6eb78](https://github.com/jackhric/r3cui/commit/dd6eb78501ccccafe03c12c2fc39ab1ef5cb25eb))
* **ui:** show the input as a centred mark in the status line ([07048ae](https://github.com/jackhric/r3cui/commit/07048ae1e1e2ab8a912bc30c92bd8f055caa61fe))
* **volume:** show power at once and follow the receiver's volume limit ([4385d54](https://github.com/jackhric/r3cui/commit/4385d54b382707d80c6a8bea4d66a02befe04e4b))
* **volume:** show volume as 0-100 of the receiver's limit ([60ee6a5](https://github.com/jackhric/r3cui/commit/60ee6a5ad1e75850f6cf678f92cbfb988b59cb5f))
* **wifi:** switch the preferred band between 2.4 and 5 GHz ([bf8d533](https://github.com/jackhric/r3cui/commit/bf8d5334c7c10bad6d207ef95fdaefd983e3b98d))


### Bug Fixes

* **backlight:** black out zones that should be off ([623dabe](https://github.com/jackhric/r3cui/commit/623dabef6279dfb1716d94cf4f91521a04d77aec))
* **backlight:** layer per-key lights over the normal frame ([d321220](https://github.com/jackhric/r3cui/commit/d3212204475477cc3e416bd213a16fbda42cdaf7))
* **backlight:** repaint all keys on wake; load library posters small ([6619a79](https://github.com/jackhric/r3cui/commit/6619a79091e2f79f40c408905cb2e7876d5657c5))
* **coreapi:** keep presses ordered without waiting out Wi-Fi stalls ([3e49007](https://github.com/jackhric/r3cui/commit/3e49007d5644dc4f1f6b9d0b6dc16ae215652e85))
* **coreapi:** send commands in order, one at a time per entity ([04b6bf3](https://github.com/jackhric/r3cui/commit/04b6bf35eb9f2e1bf0afc55ca1b307526b6519eb))
* **device:** require firmware version and treat slider as non-touchscreen ([a09f5dd](https://github.com/jackhric/r3cui/commit/a09f5ddc72a775baff08ddf7abe77e46182917e7))
* **kodi:** decide playing state from playback state only ([0dc20ae](https://github.com/jackhric/r3cui/commit/0dc20ae69ea62032a752d70a2285b70283b5de6c))
* **ui:** --shots waits for stage swaps to finish before capturing ([42a2205](https://github.com/jackhric/r3cui/commit/42a22056a633307a781beb4d834fca5754bf1b8e))
* **ui:** blur Now playing under the volume overlay in --shots; lossless --record to a .png pattern ([34988ed](https://github.com/jackhric/r3cui/commit/34988ed5747c30913fa5af6574cf8c51cf5165ba))
* **ui:** follow the receiver's power on the home screen again ([dbce924](https://github.com/jackhric/r3cui/commit/dbce924321c450485ee736ee5ff7c7f0cf3b61fe))
* **ui:** give --shots time for stage transitions to finish before capturing ([ae9ca51](https://github.com/jackhric/r3cui/commit/ae9ca5180a8b1ec985608acd23639cf98a786892))
* **ui:** keep the remote photo's transparent corners in framed shots and recordings ([178db42](https://github.com/jackhric/r3cui/commit/178db4235738475909224f5501d02a0f5133757b))
* **volume:** pace slider volume to the receiver, one command in flight ([ed8861e](https://github.com/jackhric/r3cui/commit/ed8861e9637b2b435af222ecfee2dddffcd9f8f9))
* **volume:** skip current level, wait 0.6 s for reports, use full strip ([843d07a](https://github.com/jackhric/r3cui/commit/843d07a8e97095a7157c0fe221ef1c0ef17eb04e))


### Performance Improvements

* **device:** run animations at 60 fps and hide pointer under touches ([3ccbb65](https://github.com/jackhric/r3cui/commit/3ccbb65ad099e270c63d7647be06122bc2c93023))
* **volume:** throttle slider to one command per 100 ms ([15133fb](https://github.com/jackhric/r3cui/commit/15133fb82b75125350e775a38f7cf1f2580c841e))
