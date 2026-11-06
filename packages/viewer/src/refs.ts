// Refs as the CLI writes them, worked out from the scene message alone.

import type { InstanceBase, PartData, Ref, SceneMessage } from "./protocol";

/** An instance at every depth, with its path of role names from the scene's top. */
export interface Placed {
  readonly path: readonly string[];
  readonly instance: InstanceBase;
  readonly part: PartData;
}

export function refOf(path: readonly string[]): Ref {
  return `@${path.join(".")}`;
}

export function partOf(scene: SceneMessage, instance: InstanceBase): PartData {
  const part = scene.parts[instance.part];
  if (part === undefined) {
    throw new Error(`scene message: no part data for "${instance.part}"`);
  }
  return part;
}

export function instanceAt(scene: SceneMessage, path: readonly string[]): Placed | undefined {
  let list: readonly InstanceBase[] = scene.instances;
  let found: InstanceBase | undefined;
  for (const name of path) {
    found = list.find((item) => item.name === name);
    if (found === undefined) return undefined;
    list = found.children;
  }
  return found === undefined ? undefined : { path, instance: found, part: partOf(scene, found) };
}

/** A lead of the instance at `path`, which a pin ref reaches. */
export interface LeadAt {
  readonly path: readonly string[];
  readonly lead: number;
}

/** Every lead a pin ref such as `nano.D13` reaches, through labels to any depth. A label on
 * two header pins, or a switch pin on two legs, gives more than one. */
export function leadsOf(scene: SceneMessage, pinRef: string): LeadAt[] {
  const segments = pinRef.replace(/^@/, "").split(".");
  const pin = segments.pop();
  const owner = instanceAt(scene, segments);
  if (pin === undefined || owner === undefined) return [];
  const leads = owner.part.leads[pin];
  if (leads !== undefined) {
    return leads.map((lead) => ({ path: owner.path, lead }));
  }
  return (owner.part.labels[pin] ?? []).flatMap((childPin) =>
    leadsOf(scene, [...owner.path, childPin].join("."))
  );
}

/** The ref a person sees for a lead: the outermost label that reaches it, such as
 * `@nano.D13` for lead 1 of `nano.j2`, else the pin itself, such as `@led1.anode`. */
export function leadRef(scene: SceneMessage, at: LeadAt): Ref | undefined {
  const owner = instanceAt(scene, at.path);
  if (owner === undefined) return undefined;
  const pin = Object.entries(owner.part.leads).find(([, leads]) => leads.includes(at.lead))?.[0];
  if (pin === undefined) return undefined;
  let path = at.path;
  let name = pin;
  for (;;) {
    const role = path[path.length - 1];
    const parent = instanceAt(scene, path.slice(0, -1));
    const inner = `${role}.${name}`;
    const label = parent
      ? Object.entries(parent.part.labels).find(([, pins]) => pins.includes(inner))?.[0]
      : undefined;
    if (parent === undefined || label === undefined) break;
    path = parent.path;
    name = label;
  }
  return refOf([...path, name]);
}
