import { Store } from "@core/store";
import { formatCount } from "./util/index.js";

export function activate(): Store {
  const store = new Store();
  store.set("files", 1);
  console.log(formatCount(1));
  return store;
}
