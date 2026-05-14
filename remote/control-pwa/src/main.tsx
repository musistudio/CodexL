import { createRoot } from "react-dom/client";
import { RemoteControlListApp } from "./RemoteControlListApp";
import "./react.css";

const root = document.querySelector("#root");

if (root) {
  createRoot(root).render(<RemoteControlListApp />);
}
