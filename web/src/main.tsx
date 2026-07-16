import "@astryxdesign/core/reset.css"
import "@astryxdesign/core/astryx.css"
import "@astryxdesign/theme-neutral/theme.css"
import "./shared/theme/raindrop.css"

import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import { App } from "./app/App"
import { Providers } from "./app/Providers"

const root = document.getElementById("root")
if (!root) {
  throw new Error("Raindrop root element is missing")
}

createRoot(root).render(
  <StrictMode>
    <Providers>
      <App />
    </Providers>
  </StrictMode>,
)
