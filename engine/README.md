# sim-engine

sim-engine simulates electronics from a resolved netlist and writes a deterministic run folder of traces, edges, observations, and diagnostics. It is an engine, not an application: analog circuits first, chips as behavioural black boxes at the package, and boards as assemblies only once each area is solid. Phase 1 is a Rust core for DC and transient circuits, including a 555, checked against ngspice and datasheet formulas.
