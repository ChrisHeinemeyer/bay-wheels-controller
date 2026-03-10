#!/bin/sh
# Configure git to use .githooks for pre-commit and pre-push
git config core.hooksPath .githooks
chmod +x .githooks/pre-commit .githooks/pre-push
echo "Git hooks installed. Pre-commit runs 'cargo fmt' and Prettier; pre-push runs 'cargo build --release' when src/ changes."
