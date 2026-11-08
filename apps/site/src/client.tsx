import { type ComponentType, createElement } from "react";
import { hydrateRoot } from "react-dom/client";
import "./site.css";
import { ISLANDS, type IslandName } from "./islands";

for (const element of document.querySelectorAll<HTMLElement>("[data-island]")) {
  const Component = ISLANDS[element.dataset.island as IslandName] as ComponentType<object>;
  hydrateRoot(element, createElement(Component, JSON.parse(element.dataset.props ?? "{}") as object));
}
