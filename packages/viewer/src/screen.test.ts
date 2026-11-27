import { BoxGeometry, Mesh, MeshStandardMaterial } from "three";
import { describe, expect, test } from "vitest";
import { darkGlass, decodeBase64, type ScreenBinding, showScreen } from "./screen";

function binding(): ScreenBinding {
  const material = new MeshStandardMaterial();
  new Mesh(new BoxGeometry(1, 1, 1), material);
  darkGlass([material]);
  return { materials: [material], channel: 0, shown: undefined, texture: undefined };
}

describe("showScreen", () => {
  test("decodes base64 to its bytes", () => {
    expect([...decodeBase64("Zm9vYg==")]).toEqual([102, 111, 111, 98]);
  });

  test("lights the glass with the image and updates the same texture after", () => {
    const screen = binding();
    const first = { width: 2, height: 1, rgba: "////AAAAAP8=", lit: false };
    showScreen(screen, first);
    const material = screen.materials[0] as MeshStandardMaterial;
    const texture = screen.texture;
    expect(material.emissiveMap).toBe(texture);
    expect(`#${material.emissive.getHexString()}`).toBe("#ffffff");
    expect([...(texture?.image.data as Uint8Array)]).toEqual([255, 255, 255, 0, 0, 0, 0, 255]);
    const second = { width: 2, height: 1, rgba: "AAAA//////8=", lit: true };
    showScreen(screen, second);
    expect(screen.texture).toBe(texture);
    expect(screen.shown).toBe(second);
    expect([...(texture?.image.data as Uint8Array)]).toEqual([0, 0, 0, 255, 255, 255, 255, 255]);
  });

  test("keeps the glass as it was when an image is not its size", () => {
    const screen = binding();
    showScreen(screen, { width: 4, height: 4, rgba: "AAAA", lit: false });
    expect(screen.texture).toBeUndefined();
    expect((screen.materials[0] as MeshStandardMaterial).emissiveMap).toBeNull();
  });
});
