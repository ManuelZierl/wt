export function TaskStart({ value, stepFor }) {
  return <input type={value . type === "number" ? "number" : "text"} step={stepFor(value)} />;
}
