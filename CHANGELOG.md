# Changelog

## [0.3.0](https://github.com/skillpouch/skillpouch-cli/compare/v0.2.2...v0.3.0) (2026-10-10)

### Features

* **cli:** skillpouch.exe has the SkillPouch icon and name ([b0a36b5](https://github.com/skillpouch/skillpouch-cli/commit/b0a36b5ea75ce75e481041b5b44db8079aba04b9))
* **cli:** update behind the dashboard and offer a restart; check hourly in the background ([5fcd8f0](https://github.com/skillpouch/skillpouch-cli/commit/5fcd8f0daf7b2e7989ea4ffb273cc2831fb1d754))

### Bug fixes

* **cli:** on windows, clean up the old package npm couldn't delete after an update ([dd34437](https://github.com/skillpouch/skillpouch-cli/commit/dd344372ac738c02419ae8074c8c71017d96dd0e))
* **cli:** on windows, updating pauses the background sync so its program can be replaced ([a16915e](https://github.com/skillpouch/skillpouch-cli/commit/a16915e003377e22d86a4a99d6b46b37722c9df5))

## [0.2.2](https://github.com/skillpouch/skillpouch-cli/compare/v0.2.1...v0.2.2) (2026-10-10)

### Bug fixes

* **sync:** the background sync restarts itself when the server url or installed version changes ([cb4b84b](https://github.com/skillpouch/skillpouch-cli/commit/cb4b84be04352d3a9b0848ef1890659c52ac4539))

## [0.2.1](https://github.com/skillpouch/skillpouch-cli/compare/v0.2.0...v0.2.1) (2026-10-10)

### Bug fixes

* **core:** use app.skillpouch.net as the default server ([ac8eea7](https://github.com/skillpouch/skillpouch-cli/commit/ac8eea72d74b618e08723f037e28030d9a17821f))

## [0.2.0](https://github.com/skillpouch/skillpouch-cli/compare/v0.1.1...v0.2.0) (2026-10-10)

### Features

* **cli:** update SkillPouch automatically and restart an outdated background sync ([2d65924](https://github.com/skillpouch/skillpouch-cli/commit/2d65924b5db91601c1d9bbbb7414df9c636a5e7a))
* **sync:** the background sync sends token usage every ten minutes ([6a30038](https://github.com/skillpouch/skillpouch-cli/commit/6a300385f7fb73708f7465185905a36a6e59c1b6))
* **tui:** drop Log in again and Unlink from the Account tab ([ca7ebc7](https://github.com/skillpouch/skillpouch-cli/commit/ca7ebc76ffaa6127de00d04193cee9f88b1fc9f0))
* **tui:** redraw the usage chart across the whole width from the first day with tokens ([b14bfcc](https://github.com/skillpouch/skillpouch-cli/commit/b14bfcc461636d166382c6e19aca985b4bc6071e))

### Bug fixes

* **sync:** keep what assistants had from the previous pouch when this computer is set up again ([d8fe7dd](https://github.com/skillpouch/skillpouch-cli/commit/d8fe7dde2e26eaf58b47e34672361cc020a8fdc4))
* **usage:** count OpenCode messages stored in its current session_message table ([7ba23dd](https://github.com/skillpouch/skillpouch-cli/commit/7ba23dd7bc1d0c80f41c463b5bc73975ee5b8b16))

### Performance

* **usage:** rescan only changed OpenCode messages; show the dashboard before counting ([b54fe16](https://github.com/skillpouch/skillpouch-cli/commit/b54fe1653b1e6b95d51c2b97edfaab6bd4f04347))

## [0.1.1](https://github.com/skillpouch/skillpouch-cli/compare/v0.1.0...v0.1.1) (2026-10-09)

### Bug fixes

* **sync:** refresh an expired or replaced access token and retry the sync at once ([0754018](https://github.com/skillpouch/skillpouch-cli/commit/0754018347d781019abdc4750feddcff79609fe0))
