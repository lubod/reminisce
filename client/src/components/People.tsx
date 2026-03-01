import { useEffect } from "react";
import { observer } from "mobx-react-lite";
import { useStore } from "../stores/RootStore";
import { PersonGallery } from "./PersonGallery";
import { PersonDetail } from "./PersonDetail";
import { useParams, useNavigate } from "react-router-dom";

export const People = observer(() => {
    const { personStore } = useStore();
    const { personId } = useParams<{ personId: string }>();
    const navigate = useNavigate();

    useEffect(() => {
        if (personId) {
            const id = parseInt(personId);
            if (!isNaN(id)) {
                if (personStore.selectedPerson?.id !== id) {
                    personStore.fetchPerson(id);
                }
            } else {
                navigate("/people");
            }
        } else {
            if (personStore.selectedPerson) {
                personStore.clearSelection();
            }
        }
    }, [personId, personStore, navigate]);

    return (
        <div>
            {personId ? (
                <PersonDetail />
            ) : (
                <PersonGallery />
            )}
        </div>
    );
});
