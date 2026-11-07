import { Disposable, Emitter } from "./event";

export class Store implements Disposable {
  private readonly onChange = new Emitter<string>();
  private readonly data = new Map<string, number>();

  set(key: string, value: number): void {
    this.data.set(key, value);
    this.onChange.fire(key);
  }

  dispose(): void {
    this.onChange.dispose();
  }
}
