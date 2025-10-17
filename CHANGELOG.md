# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-10-17

### Added
- Initial release extracted from arsync project
- Semaphore: Async semaphore for bounding concurrency
  - Lock-free fast path using atomics
  - FIFO waiter queue for fairness
  - RAII permit guards
- Condvar: Async condition variable for task notification
  - Wait/notify semantics
  - Support for notify_one() and notify_all()
- WaiterQueue: Internal FIFO waiter queue implementation
- Comprehensive documentation and design docs

### Notes
- Extracted from [arsync](https://github.com/jmalicki/arsync)
- This is a baby step towards contributing to upstream [compio](https://github.com/compio-rs/compio)

[0.1.0]: https://github.com/jmalicki/compio-sync/releases/tag/v0.1.0

