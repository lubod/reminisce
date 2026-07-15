import axios from "axios";

const instance = axios.create({
  baseURL: "/api", // All requests will be prefixed with /api
  withCredentials: true,
});

// Add a response interceptor to handle 401 errors
instance.interceptors.response.use(
  (response) => {
    return response;
  },
  (error) => {
    // Check if the error is a 401 Unauthorized — but not from the login endpoint itself
    // (a 401 there means wrong credentials, not an expired session)
    if (error.response && error.response.status === 401 && !error.config?.url?.includes("user-login")) {
      // Redirect to the login page
      window.location.href = "/login";
    }
    return Promise.reject(error);
  }
);

export default instance;
