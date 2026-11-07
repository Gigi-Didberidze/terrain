export interface Disposable {
  dispose(): void;
}

export class Emitter<T> implements Disposable {
  private listeners: ((e: T) => void)[] = [];
  fire(e: T): void {
    for (const l of this.listeners) l(e);
  }
  dispose(): void {
    this.listeners = [];
  }
}
