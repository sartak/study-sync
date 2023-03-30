study-sync is a service designed to run alongside [RetroArch](https://www.retroarch.com) on a device like an [RG351M](https://anbernic.com/products/anbernic-rg351m) to provide the following features:

- Track game start and end times and sync them to an external service ("intake"). Big fan of Quantified Self
- Sync all screenshots, organized by game, to an external service ("study"). I [turn these](https://shawn.dev/2022/03/one-million-anki-reviews.html) into [Anki](https://apps.ankiweb.net) flashcards
- Sync all save states and save files, each bundled with the latest screenshot for identification, to an external service ("saves")
- Allow restarting study-sync, or the entire device, without losing any state; including graceful shutdown on SIGTERM/ctrl-c
- Fully tolerate being offline (or on an unreliable connection) for extended periods, and automatically sync everything when back online
- Notify player of progress and errors by blinking the device's LED

It's written in async Rust. The various components are:

- `orchestrator` is the event dispatcher
- `server` creates an HTTP listener to receive events from RetroArch and the operating system
- `watcher` watches the filesystem for new screenshots and saves
- `database` uses SQLite to track game starts and ends, and sync status
- `intake` syncs game starts and ends to an "intake" service
- `screenshots` syncs screenshots to a "study" service
- `saves` syncs save states to a "saves" service
- `notify` writes to a GPIO pin which controls an LED to indicate progress and errors
