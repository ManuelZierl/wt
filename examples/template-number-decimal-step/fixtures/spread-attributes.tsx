export function TaskStart({ value, props }) {
  return <input {...props} type={value . type === "number" ? "number" : "text"} />;
}
