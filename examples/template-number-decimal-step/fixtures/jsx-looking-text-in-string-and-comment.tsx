const text = "<input type={value . type === \"number\" ? \"number\" : \"text\"} />";
// <input type={value . type === "number" ? "number" : "text"} />
export function TaskStart() {
  return <div>{text}</div>;
}
