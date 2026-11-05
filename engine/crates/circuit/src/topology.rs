use sim_core::diagnostic::Diagnostic;
use sim_core::name::Ref;
use sim_core::netlist::Netlist;

use crate::element;

pub(crate) fn floating_nets(netlist: &Netlist) -> Vec<Diagnostic> {
    let count = netlist.nets().len();
    let mut parent: Vec<usize> = (0..count).collect();
    for (_, part) in netlist.parts() {
        let pins = part.model.pins();
        for &(left, right) in element::dc_paths(&part.model) {
            unite(&mut parent, pins[left].index(), pins[right].index());
        }
    }
    let ground = find(&mut parent, 0);
    let mut out = Vec::new();
    for (id, name) in netlist.nets() {
        if find(&mut parent, id.index()) != ground {
            out.push(Diagnostic::NetFloating {
                net: Ref::Net(name.clone()),
            });
        }
    }
    out
}

fn find(parent: &mut [usize], mut index: usize) -> usize {
    let mut root = index;
    while parent[root] != root {
        root = parent[root];
    }
    while parent[index] != root {
        let next = parent[index];
        parent[index] = root;
        index = next;
    }
    root
}

fn unite(parent: &mut [usize], left: usize, right: usize) {
    let left = find(parent, left);
    let right = find(parent, right);
    if left != right {
        parent[right] = left;
    }
}
