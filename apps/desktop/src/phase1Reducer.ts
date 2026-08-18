export interface IpcState {
  schema_version: number;
  sequence: number;
  readiness: string;
  session: string;
  engine: string;
  delivery: string;
  transcript: string | null;
  error_code: string | null;
}

export interface FakeFlowResponse {
  schema_version: number;
  states: IpcState[];
}

export interface Phase1State {
  sequence: number;
  readiness: string;
  session: string;
  engine: string;
  delivery: string;
  transcript: string | null;
  errorCode: string | null;
}

export const initialState: Phase1State = {
  sequence: 0,
  readiness: "starting",
  session: "idle",
  engine: "unavailable",
  delivery: "result_view_only",
  transcript: null,
  errorCode: null,
};

export type Phase1Action =
  | { type: "flow"; response: FakeFlowResponse }
  | { type: "ipc_failure" };

export function phase1Reducer(state: Phase1State, action: Phase1Action): Phase1State {
  if (action.type === "ipc_failure") {
    return { ...state, session: "failed", errorCode: "ipc_failure" };
  }
  if (action.response.schema_version !== 1) {
    return { ...state, session: "failed", errorCode: "ipc_schema_unsupported" };
  }
  return action.response.states.reduce((current, incoming) => {
    if (incoming.schema_version !== 1 || incoming.sequence < current.sequence) {
      return current;
    }
    return {
      sequence: incoming.sequence,
      readiness: incoming.readiness,
      session: incoming.session,
      engine: incoming.engine,
      delivery: incoming.delivery,
      transcript: incoming.transcript ?? current.transcript,
      errorCode: incoming.error_code,
    };
  }, state);
}