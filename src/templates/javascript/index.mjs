export default async function handler(request) {
  console.log("{{FUNCTION_ID}}: received", request.method, request.path);

  const echoed = request.body && typeof request.body === "object"
    ? request.body
    : null;

  return {
    status: 200,
    headers: { "content-type": "application/json" },
    body: {
      ok: true,
      functionId: "{{FUNCTION_ID}}",
      functionName: "{{FUNCTION_NAME}}",
      method: request.method,
      path: request.path,
      echoed,
    },
  };
}