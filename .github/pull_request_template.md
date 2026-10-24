## Description
Please include a summary of the change and which issue it addresses or resolves. Include relevant motivation and context.

Fixes #(issue)

## Type of Change
- [ ] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] Documentation update
- [ ] Packaging / Build system improvement

## Quality Gate Checklist
- [ ] My code adheres to the project's code style (`cargo fmt --all -- --check` passes).
- [ ] I have run `cargo clippy --workspace --all-targets -- -D warnings` with zero warnings.
- [ ] All automated workspace tests pass (`cargo test --workspace --all-targets`).
- [ ] I have verified that no cleartext secrets/passwords are persisted or logged.
- [ ] I have verified that non-root security enforcement remains intact.
- [ ] I have updated relevant documentation where appropriate (`docs/`, `README.md`, `CHANGELOG.md`).
- [ ] Desktop and packaging files have been validated (`desktop-file-validate`).
