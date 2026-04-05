# Changelog

## [0.1.1-beta](https://github.com/cntm-labs/chorus/compare/chorus-core-v0.1.0-beta...chorus-core-v0.1.1-beta) (2026-04-05)


### Features

* **core:** add Chorus client with builder pattern, template rendering, and OTP ([14efd0c](https://github.com/cntm-labs/chorus/commit/14efd0c5e979f2901237d8720b6f9c9961cc39b6))
* **core:** add ChorusError, types (SmsMessage, EmailMessage, SendResult, Channel, DeliveryStatus) ([bfdc652](https://github.com/cntm-labs/chorus/commit/bfdc65248a8b1c3a91e5af94e8d0e4d9a975249c))
* **core:** add SmsSender and EmailSender traits ([00515f2](https://github.com/cntm-labs/chorus/commit/00515f2d87422f9b05da1a9d9cfbe3c4329d696d))
* **core:** add Template engine with {{variable}} rendering ([35ae40e](https://github.com/cntm-labs/chorus/commit/35ae40e8dc4f7d1ff48b15b5130238c920e64509))
* **core:** add WaterfallRouter with email-first/SMS-fallback and multi-provider failover ([a8d9f14](https://github.com/cntm-labs/chorus/commit/a8d9f1476cdde960e8db6de39909454db8051d62))
* Phase 1 — Core library (chorus-core + chorus-providers) ([4ffd5ca](https://github.com/cntm-labs/chorus/commit/4ffd5ca383fbb6feec1a3f79e1eebd45fc0da4b4))


### Bug Fixes

* correct formatting in types.rs to match stable rustfmt ([161008a](https://github.com/cntm-labs/chorus/commit/161008a3ff27d4be972ba7ce71b4408538ac721d))
