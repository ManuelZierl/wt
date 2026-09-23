export function TaskStart({ value, onChange }) {
  return <input onChange={(event) => onChange(event)} type={value . type === "number" ? "number" : "text"} />;
}
