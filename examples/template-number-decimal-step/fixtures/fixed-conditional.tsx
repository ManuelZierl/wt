export function TaskStart({ value }) {
  return <input type={value . type === "number" ? "number" : "text"} step={value . type === "number" ? "any" : undefined} />;
}
