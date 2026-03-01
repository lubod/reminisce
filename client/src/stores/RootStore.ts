import { createContext, useContext } from "react";
import { AuthStore } from "./AuthStore";
import { MediaStore } from "./MediaStore";
import { UIStore } from "./UIStore";
import { StatsStore } from "./StatsStore";
import { PersonStore } from "./PersonStore";
import { LabelStore } from "./LabelStore";

export class RootStore {
    authStore: AuthStore;
    mediaStore: MediaStore;
    uiStore: UIStore;
    statsStore: StatsStore;
    personStore: PersonStore;
    labelStore: LabelStore;

    constructor() {
        this.authStore = new AuthStore(this);
        this.mediaStore = new MediaStore(this);
        this.uiStore = new UIStore(this);
        this.statsStore = new StatsStore(this);
        this.personStore = new PersonStore(this);
        this.labelStore = new LabelStore(this);
    }
}

export const rootStore = new RootStore();
export const StoreContext = createContext(rootStore);

export const useStore = () => {
    return useContext(StoreContext);
};
