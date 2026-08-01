import axios from "axios";

const instance = axios.create({
  baseURL: "/api", // All requests will be prefixed with /api
  withCredentials: true, // send the httpOnly session cookie
});

// Authentication is delegated to the backend's httpOnly, SameSite session cookie
// (set on login). No token is read from or written to localStorage, so JWTs are not
// exposed to XSS/the DOM.
instance.interceptors.request.use(
  (config) => config,
  (error) => {
    return Promise.reject(error);
  }
);

// Add a response interceptor to handle 401 errors
instance.interceptors.response.use(
  (response) => {
    return response;
  },
  (error) => {
    // Check if the error is a 401 Unauthorized — but not from the login endpoint itself
    // (a 401 there means wrong credentials, not an expired session)
    if (
      error.response &&
      error.response.status === 401 &&
      !error.config?.url?.includes("user-login") &&
      !error.config?.url?.includes("/auth/me") &&
      window.location.pathname !== "/login"
    ) {
      // Session cookie expired/invalid — redirect to the login page
      window.location.href = "/login";
    }
    return Promise.reject(error);
  }
);

export default instance;
