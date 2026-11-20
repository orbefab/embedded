// Adapted from maker-sandbox (MIT, Copyright 2026 Alwurts).
import { useEffect, useMemo } from "react";
import { BoxGeometry, CatmullRomCurve3, type Material, Quaternion, TubeGeometry, Vector3 } from "three";
import type { DrawnWire } from "./build";
import { type LookKit, smoothCopy } from "./look-materials";
import { along, HOUSING_MM, INSERT_MM, WIRE_RADIUS_MM, type WireEnd } from "./wire-geometry";

const HOUSING_WIDTH_MM = 2.5;
const housingGeometry = new BoxGeometry(HOUSING_WIDTH_MM, HOUSING_MM, HOUSING_WIDTH_MM);
const housingHull = smoothCopy(housingGeometry);
const Y = new Vector3(0, 1, 0);

/** A female housing over the lead's tip. */
function Housing({ end, material, kit }: { end: WireEnd; material: Material; kit: LookKit }) {
  const turn = useMemo(() => new Quaternion().setFromUnitVectors(Y, new Vector3(...end.axis)), [end]);
  const shadows = kit.look.shadows === "map";
  return (
    <mesh
      castShadow={shadows}
      geometry={housingGeometry}
      material={material}
      position={along(end.at, end.axis, HOUSING_MM / 2 - INSERT_MM)}
      quaternion={turn}
      receiveShadow={shadows}
    >
      {kit.hull === null ? null : <mesh geometry={housingHull} material={kit.hull} raycast={() => {}} />}
    </mesh>
  );
}

export function WireMesh({ wire, kit }: { wire: DrawnWire; kit: LookKit }) {
  const geometry = useMemo(() => {
    const curve = new CatmullRomCurve3(wire.path.points.map((point) => new Vector3(...point)));
    curve.curveType = "centripetal";
    return new TubeGeometry(curve, Math.max(wire.path.points.length * 2, 8), WIRE_RADIUS_MM, 8, false);
  }, [wire]);
  useEffect(() => () => geometry.dispose(), [geometry]);
  const shadows = kit.look.shadows === "map";
  return (
    <group>
      <mesh castShadow={shadows} geometry={geometry} material={kit.wire(wire.colour)} receiveShadow={shadows}>
        {kit.hull === null ? null : <mesh geometry={geometry} material={kit.hull} raycast={() => {}} />}
      </mesh>
      <Housing end={wire.path.ends[0]} kit={kit} material={kit.housing} />
      <Housing end={wire.path.ends[1]} kit={kit} material={kit.housing} />
    </group>
  );
}
