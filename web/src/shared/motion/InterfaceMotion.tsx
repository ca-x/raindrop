import { useLayoutEffect, type ReactNode } from "react"

export function InterfaceMotion({ children }: { children: ReactNode }) {
  useLayoutEffect(() => {
    const root = document.documentElement
    const setInput = (input: "pointer" | "keyboard") => {
      if (root.dataset.raindropInput !== input) root.dataset.raindropInput = input
    }
    const pointer = () => setInput("pointer")
    const keyboard = () => setInput("keyboard")
    const click = (event: MouseEvent) => {
      if (event.detail === 0) keyboard()
    }

    document.addEventListener("pointerdown", pointer, true)
    document.addEventListener("keydown", keyboard, true)
    document.addEventListener("click", click, true)
    return () => {
      document.removeEventListener("pointerdown", pointer, true)
      document.removeEventListener("keydown", keyboard, true)
      document.removeEventListener("click", click, true)
      delete root.dataset.raindropInput
    }
  }, [])

  return children
}
