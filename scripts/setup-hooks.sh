#!/bin/sh
# Configure git to use .githooks for pre-commit and pre-push
git config core.hooksPath .githooks
chmod +x .githooks/pre-commit .githooks/pre-push
echo "Git hooks installed. Pre-commit will run 'cargo fmt', pre-push will run 'cargo run'."
