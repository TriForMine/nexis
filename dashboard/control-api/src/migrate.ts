import { migrate } from "./db";

await migrate();
console.log("control-api migrations applied");
