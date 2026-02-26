import { redirect } from "next/navigation";

export default function HomeRedirectPage() {
  redirect("/getting-started/quickstart/");
}
