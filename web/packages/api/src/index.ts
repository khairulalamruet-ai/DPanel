import { QueryClient } from "@tanstack/react-query";

export type * from "./schema";
export * from "./client";
export * from "./pending";
export * from "./mocks/handlers";
export * from "./mocks/browser";
export { QueryClient, QueryClientProvider, useQuery, useMutation } from "@tanstack/react-query";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60 * 5, // 5 minutes
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
});
