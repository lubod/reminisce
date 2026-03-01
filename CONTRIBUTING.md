# Contributing to Reminisce

Thank you for your interest in contributing to Reminisce! We welcome contributions from everyone.

## Getting Started

1.  **Fork the repository** on GitHub.
2.  **Clone your fork** locally:
    ```bash
    git clone https://github.com/lubod/reminisce.git
    cd reminisce
    ```
3.  **Set up the development environment**:
    Follow the instructions in [DEV_SETUP.md](DEV_SETUP.md) to get the Docker containers and local server running.
    ```bash
    ./dev start
    ```

## Development Workflow

1.  Create a new branch for your feature or bugfix:
    ```bash
    git checkout -b feature/my-amazing-feature
    ```
2.  Make your changes.
3.  **Run tests** to ensure you haven't broken anything:
    ```bash
    ./dev test
    ```
4.  Commit your changes with clear messages.
5.  Push to your fork and submit a **Pull Request**.

## Code Style

- **Rust**: We use `rustfmt`. Please run `cargo fmt` before committing.
- **Frontend**: We use Prettier/ESLint. Run `npm run lint` in the `client/` directory.

## Reporting Bugs

Please check existing issues before opening a new one. If you find a bug, please include:
- Steps to reproduce
- Expected behavior vs. actual behavior
- Logs (if relevant)
- Environment details (OS, Docker version)
