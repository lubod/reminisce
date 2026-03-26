import { makeAutoObservable, runInAction } from "mobx";
import { RootStore } from "./RootStore";
import axios from "../api/axiosConfig";

export interface Person {
    id: number;
    name: string | null;
    face_count: number;
    representative_face_hash: string | null;
    representative_face_deviceid: string | null;
    representative_face_id: number | null;
    representative_bbox: number[] | null;
    representative_face_url: string | null;
    created_at: string;
    updated_at: string;
    thumbnailUrl?: string;
}

export interface PersonImage {
    hash: string;
    deviceid: string;
    name: string;
    created_at: string;
    bbox: number[];
    confidence: number;
    thumbnailUrl?: string;      // face-cropped, used in the grid
    fullThumbnailUrl?: string;  // full image, used in lightbox
    thumbnail_url?: string;
    face_id: number;
    place?: string;
    starred?: boolean;
}

export interface PersonsResponse {
    persons: Person[];
    total: number;
}

export interface PersonResponse {
    person: Person;
}

export interface PersonImagesResponse {
    images: PersonImage[];
    total: number;
}

export class PersonStore {
    rootStore: RootStore;
    persons: Person[] = [];
    selectedPerson: Person | null = null;
    personImages: PersonImage[] = [];
    isLoading: boolean = false;
    isLoadingImages: boolean = false;
    isLoadingMoreImages: boolean = false;
    page: number = 1;
    limit: number = 50;
    hasMore: boolean = true;
    total: number = 0;

    imagesOffset: number = 0;
    imagesLimit: number = 60;
    imagesTotal: number = 0;
    imagesHasMore: boolean = false;

    constructor(rootStore: RootStore) {
        makeAutoObservable(this);
        this.rootStore = rootStore;
    }

    fetchPersons = async (reset: boolean = false) => {
        if (this.isLoading) return;
        if (!reset && !this.hasMore) return;

        this.isLoading = true;
        if (reset) {
            this.page = 1;
            this.hasMore = true;
            this.persons = [];
        }

        try {
            const response = await axios.get<PersonsResponse>(`/persons?page=${this.page}&limit=${this.limit}`);

            const personsWithThumbnails = response.data.persons.map(person => ({
                ...person,
                thumbnailUrl: person.representative_face_url ? this.getAuthenticatedUrl(person.representative_face_url) : undefined
            }));

            runInAction(() => {
                if (reset) {
                    this.persons = personsWithThumbnails;
                } else {
                    this.persons = [...this.persons, ...personsWithThumbnails];
                }
                this.total = response.data.total;
                this.hasMore = this.persons.length < this.total;
                if (this.hasMore) {
                    this.page += 1;
                }
            });
        } catch (error) {
            console.error("Failed to fetch persons", error);
            this.rootStore.uiStore.setError("Failed to fetch persons");
        } finally {
            runInAction(() => {
                this.isLoading = false;
            });
        }
    };

    fetchPerson = async (id: number) => {
        this.isLoading = true;
        try {
            const response = await axios.get<PersonResponse>(`/persons/${id}`);
            let person = {
                ...response.data.person,
                thumbnailUrl: response.data.person.representative_face_url 
                    ? this.getAuthenticatedUrl(response.data.person.representative_face_url) 
                    : undefined
            };

            runInAction(() => {
                this.selectedPerson = person;
            });
            
            // Also fetch images for this person
            await this.fetchPersonImages(id);
            
        } catch (error) {
            console.error(`Failed to fetch person ${id}`, error);
            this.rootStore.uiStore.setError("Failed to fetch person");
        } finally {
            runInAction(() => {
                this.isLoading = false;
            });
        }
    };

    selectPerson = async (person: Person) => {
        this.selectedPerson = person;
        await this.fetchPersonImages(person.id);
    };

    fetchPersonImages = async (personId: number, reset: boolean = true) => {
        if (reset) {
            this.isLoadingImages = true;
            this.imagesOffset = 0;
        } else {
            this.isLoadingMoreImages = true;
        }
        try {
            const offset = reset ? 0 : this.imagesOffset;
            const response = await axios.get<PersonImagesResponse>(
                `/persons/${personId}/images?limit=${this.imagesLimit}&offset=${offset}`
            );

            const imagesWithThumbnails = response.data.images.map(image => ({
                ...image,
                thumbnailUrl: this.getAuthenticatedUrl(`/api/face/${image.face_id}/thumbnail`),
                fullThumbnailUrl: image.thumbnail_url ? this.getAuthenticatedUrl(image.thumbnail_url) : undefined,
            }));

            runInAction(() => {
                if (reset) {
                    this.personImages = imagesWithThumbnails;
                } else {
                    this.personImages = [...this.personImages, ...imagesWithThumbnails];
                }
                this.imagesTotal = response.data.total;
                this.imagesOffset = offset + imagesWithThumbnails.length;
                this.imagesHasMore = this.imagesOffset < response.data.total;
            });
        } catch (error) {
            console.error(`Failed to fetch images for person ${personId}`, error);
            this.rootStore.uiStore.setError("Failed to fetch person images");
        } finally {
            runInAction(() => {
                this.isLoadingImages = false;
                this.isLoadingMoreImages = false;
            });
        }
    };

    loadMorePersonImages = async () => {
        if (!this.selectedPerson || !this.imagesHasMore || this.isLoadingMoreImages) return;
        await this.fetchPersonImages(this.selectedPerson.id, false);
    };

    updatePersonName = async (personId: number, name: string) => {
        try {
            await axios.put(`/persons/${personId}/name`, { name });

            runInAction(() => {
                const person = this.persons.find(p => p.id === personId);
                if (person) {
                    person.name = name;
                }
                if (this.selectedPerson?.id === personId) {
                    this.selectedPerson.name = name;
                }
            });
        } catch (error) {
            console.error("Failed to update person name", error);
            this.rootStore.uiStore.setError("Failed to update person name");
        }
    };

    setRepresentativeFace = async (personId: number, faceId: number) => {
        try {
            await axios.put(`/persons/${personId}/representative_face`, { face_id: faceId });
            // Refresh so thumbnail URL updates
            await this.fetchPerson(personId);
        } catch (error) {
            console.error("Failed to set representative face", error);
            this.rootStore.uiStore.setError("Failed to set representative face");
        }
    };

    mergePersons = async (sourceIds: number[], targetId: number) => {
        try {
            await axios.post('/persons/merge', {
                source_person_ids: sourceIds,
                target_person_id: targetId
            });

            // Refresh persons list and the surviving target person
            await Promise.all([this.fetchPersons(), this.fetchPerson(targetId)]);
        } catch (error) {
            console.error("Failed to merge persons", error);
            this.rootStore.uiStore.setError("Failed to merge persons");
        }
    };

    clearSelection = () => {
        // Clean up image thumbnail URLs when going back to person list
        this.personImages.forEach(image => {
            if (image.thumbnailUrl?.startsWith('blob:')) {
                URL.revokeObjectURL(image.thumbnailUrl);
            }
        });

        this.selectedPerson = null;
        this.personImages = [];
        this.imagesOffset = 0;
        this.imagesTotal = 0;
        this.imagesHasMore = false;
    };

    getAuthenticatedUrl = (baseUrl: string) => {
        const token = this.rootStore.authStore.token;
        if (!token) return baseUrl;
        const separator = baseUrl.includes('?') ? '&' : '?';
        return `${baseUrl}${separator}token=${token}`;
    };

    cleanup = () => {
        // Clean up all thumbnail URLs on unmount
        this.persons.forEach(person => {
            if (person.thumbnailUrl?.startsWith('blob:')) {
                URL.revokeObjectURL(person.thumbnailUrl);
            }
        });
        this.personImages.forEach(image => {
            if (image.thumbnailUrl?.startsWith('blob:')) {
                URL.revokeObjectURL(image.thumbnailUrl);
            }
        });
    };
}
