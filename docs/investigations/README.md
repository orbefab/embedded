# Investigations

These run before any code is migrated. Each one reads and proposes; it doesn't migrate code or pick the final answer. An investigation may challenge decided items in [`../decisions.md`](../decisions.md), with reasons.

## 1. Entry points and usage (big picture)
How people and agents use everything: the CLI, an MCP server and where it lives, the local app's shape, the web sandbox, and how they fit together. This one is decided as a whole, and the others should fit it.

## 2. Visualizer architecture and framework
One shared visualization engine with a web shell (the sandbox) and a local-computer shell, and which framework(s) to use.

## 3. Parts organization and catalog
A per-part layout with sim model, specs, 3D and 2D; how parts nest; where large 3D assets live; and the catalog design, to be filled starting with the uno board.

## 4. Scene format
How scenes are written, with reusable components (like JSX for UI). Options to compare include JSON (today), JSON with objects/references, HTML- or XML-like markup, a code-based format, or something else.
