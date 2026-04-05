import { createContext, useContext } from "react";

export type SettingsTab = "providers" | "ai" | "voice";

const SettingsContext = createContext<(tab?: SettingsTab) => void>(() => {});

export const SettingsProvider = SettingsContext.Provider;
export const useOpenSettings = () => useContext(SettingsContext);
