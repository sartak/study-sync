study-sync is a service designed to run alongside [RetroArch](https://www.retroarch.com) to provide the following features:

- Track game start and end times and sync them to an external service ("intake")
- Save all screenshots, organized by game, and sync them to an external service ("study")
- Save all save states and save files, each bundled with the latest screenshot for identification, and sync them to an external service ("saves")
- Allow restarting study-sync, or the entire device, without losing any state
- Fully tolerate being offline (or on an unreliable connection) for extended periods, and automatically sync everything when back online

It's written in async Rust. The various components are:

- `orchestrator` is the event dispatcher
- `server` creates an HTTP listener to receive events from RetroArch and the operating system
- `watch` watches the filesystem for new screenshots
- `database` uses SQLite to track game starts and ends, and sync status
- `intake` syncs game starts and ends to an "intake" service
- `screenshots` syncs screenshots to a "study" service
- `saves` syncs save states to a "saves" service
