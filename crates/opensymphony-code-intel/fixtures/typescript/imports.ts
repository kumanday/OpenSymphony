import { readFile } from "node:fs/promises";

type Loader = (path: string) => Promise<string>;

export const loadText: Loader = async (path) => {
    const data = await readFile(path, "utf8");
    return data.trim();
};
