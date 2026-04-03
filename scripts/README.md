# Arcadestr Scripts

This directory contains utility scripts for the Arcadestr project.

## Available Scripts

### analyze-patterns

A powerful code analysis tool that searches the codebase for recurring patterns, similar implementations, and refactoring opportunities.

#### Usage

```bash
./scripts/analyze-patterns [OPTIONS]
```

#### Options

| Option | Description |
|--------|-------------|
| `--pattern=<name>` | Pattern to search for (see list below) |
| `--language=<lang>` | Filter by language (rust, go, py, js, ts, etc.) |
| `--depth=<level>` | Search depth: shallow, medium (default), deep |
| `--output=<format>` | Output format: text (default), json, markdown |
| `--list-patterns` | List all available predefined patterns |
| `--help` | Show help message |

#### Examples

```bash
# List all available patterns
./scripts/analyze-patterns --list-patterns

# Analyze error handling patterns in Rust code
./scripts/analyze-patterns --pattern=error-handling --language=rust

# Analyze async patterns with JSON output
./scripts/analyze-patterns --pattern=async-patterns --output=json

# Deep search for mutex patterns with markdown output
./scripts/analyze-patterns --pattern=mutex-patterns --depth=deep --output=markdown

# Custom pattern search
./scripts/analyze-patterns --pattern="Arc<Mutex" --depth=medium
```

#### Predefined Patterns

**Rust-Specific Patterns:**

| Pattern | Description |
|---------|-------------|
| `error-handling` | Result types, Error types, thiserror, anyhow |
| `async-patterns` | async/await, tokio, futures |
| `singleton` | lazy_static, once_cell, Arc<Mutex> |
| `factory` | Factory pattern implementations |
| `builder` | Builder pattern implementations |
| `trait-patterns` | trait definitions and implementations |
| `mutex-patterns` | Mutex, RwLock usage patterns |
| `serialization` | serde, Serialize, Deserialize |
| `logging` | tracing, log macros |
| `testing` | Test modules and attributes |
| `unsafe` | unsafe code blocks |
| `ffi` | Foreign function interface patterns |

**General Patterns:**

| Pattern | Description |
|---------|-------------|
| `todo` | TODO, FIXME, XXX, HACK, BUG comments |
| `documentation` | Documentation comments (///, //!) |
| `feature-gates` | Conditional compilation (#[cfg(...)]) |
| `public-api` | Public function/struct definitions |
| `imports` | Import statements |

#### Output Formats

**Text (Default):**
Human-readable report with:
- Pattern summary (occurrences, files, similarity)
- List of files containing the pattern
- Refactoring suggestions

**JSON:**
Structured data for programmatic processing:
```json
{
  "pattern": "error-handling",
  "regex": "(Result<|Error|...)",
  "occurrences": 827,
  "file_count": 42,
  "similarity": "high",
  "files": [...],
  "suggestions": "..."
}
```

**Markdown:**
Formatted documentation suitable for sharing or including in docs.

#### Search Depth

- **shallow**: Current directory only
- **medium**: Source directories (core/src, desktop/src, app/src, web/src)
- **deep**: Entire repository including all subdirectories

#### Custom Patterns

You can use any regex pattern directly:

```bash
./scripts/analyze-patterns --pattern="your-regex-here"
```

#### Integration with Development Workflow

The `analyze-patterns` command is useful for:

1. **Code Reviews**: Identify inconsistent implementations
2. **Refactoring**: Find patterns that should be abstracted
3. **Documentation**: Generate pattern usage reports
4. **Quality Assurance**: Check for anti-patterns or problematic code

#### Sample Output

```
==========================================
Pattern Analysis Report
==========================================

Pattern: error-handling
Regex:   (Result<|Error|thiserror|anyhow|\.expect\(|\.unwrap\()

Summary
-------
Occurrences: 827
Files:       42
Similarity:  high

Files Containing Pattern
------------------------
  core/src/auth/account_manager.rs                 (51 occurrences)
  core/src/nostr.rs                                  (77 occurrences)
  ...

Refactoring Suggestions
---------------------
- Consider creating a centralized error type to reduce duplication
- Evaluate using anyhow for application code vs thiserror for libraries

==========================================
```

## Contributing

When adding new scripts:

1. Make the script executable: `chmod +x scripts/your-script`
2. Add documentation to this README
3. Include a `--help` option in your script
4. Follow the existing script structure for consistency
