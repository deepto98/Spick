import "@testing-library/jest-dom/vitest";

const localValues = new Map<string, string>();
const localStorageMock: Storage = {
  get length() {
    return localValues.size;
  },
  clear() {
    localValues.clear();
  },
  getItem(key) {
    return localValues.get(key) ?? null;
  },
  key(index) {
    return [...localValues.keys()][index] ?? null;
  },
  removeItem(key) {
    localValues.delete(key);
  },
  setItem(key, value) {
    localValues.set(key, value);
  },
};

Object.defineProperty(window, "localStorage", {
  configurable: true,
  value: localStorageMock,
});
