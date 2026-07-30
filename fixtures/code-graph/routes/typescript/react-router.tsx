import { Route, createBrowserRouter } from "react-router-dom";
import { AccountPage as AccountAlias } from "./AccountPage";
import { UserPage } from "./UserPage";

export const routes = (
  <Route path="/accounts/:accountId" element={<AccountAlias />} />
);

export const router = createBrowserRouter([
  {
    path: "/users/:userId",
    Component: UserPage,
    loader: loadUser,
  },
]);

export function loadUser() {}
