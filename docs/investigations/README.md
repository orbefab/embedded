# Investigations

These run before any code is migrated. Each one reads and proposes; it doesn't migrate code or pick the final answer. An investigation may challenge decided items in [`../decisions.md`](../decisions.md), with reasons.

## 1. Entry points and usage (big picture)
How people and agents use everything: the CLI, an MCP server and where it lives, the local app's shape, the web sandbox, and how they fit together. Decided: see the entry points entry in [`../decisions.md`](../decisions.md).

## 2. Visualizer architecture and framework
One shared visualization engine with a web shell (the sandbox) and a local-computer shell, and which framework(s) to use. Decided: see the visualizer entry in [`../decisions.md`](../decisions.md).

## 3. Parts organization and catalog
A per-part layout with sim model, specs and 3D; how parts nest; where large 3D assets live; and the catalog design, to be filled starting with the nano board. Decided: see the parts, catalog and nano board entries in [`../decisions.md`](../decisions.md).

## 4. Scene format
How scenes are written, with reusable components (like JSX for UI). Options to compare include JSON (today), JSON with objects/references, HTML- or XML-like markup, a code-based format, or something else. Decided: see the scenes and checking entries in [`../decisions.md`](../decisions.md).
