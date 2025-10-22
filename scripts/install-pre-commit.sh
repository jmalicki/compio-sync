#!/bin/bash
# Install pre-commit hook for compio-sync

set -e

echo "Installing pre-commit hook for compio-sync..."

# Create the pre-commit hook
cat > .git/hooks/pre-commit << 'EOF'
#!/bin/bash
# Pre-commit hook for compio-sync
# Automatically formats and lints code before commits

set -e

echo "🔍 Running pre-commit checks..."

# Format all Rust code
echo "📝 Formatting code with cargo fmt..."
cargo fmt --all

# Check if formatting changed anything
if ! git diff --quiet; then
    echo "❌ Code was not properly formatted. Please run 'cargo fmt --all' and commit again."
    echo "The following files need formatting:"
    git diff --name-only
    exit 1
fi

# Run clippy lints with strict settings
echo "🔍 Running clippy lints..."
cargo clippy --all-targets --all-features -- -D warnings

# Run tests to ensure nothing is broken
echo "🧪 Running tests..."
cargo test --all-targets

echo "✅ All pre-commit checks passed!"
EOF

# Make it executable
chmod +x .git/hooks/pre-commit

echo "✅ Pre-commit hook installed successfully!"
echo "The hook will now run on every commit to ensure code quality."
