import { ImageResponse } from "next/og";

export const dynamic = "force-static";

export const size = {
  width: 32,
  height: 32,
};

export const contentType = "image/png";

export default function Icon() {
  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          display: "flex",
          padding: "2px",
          boxSizing: "border-box",
          background: "#050608",
          borderRadius: "8px",
        }}
      >
        <div
          style={{
            width: "100%",
            height: "100%",
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
            borderRadius: "6px",
            border: "1px solid #1f242a",
            background: "#0c0d0e",
          }}
        >
          <div
            style={{
              height: "7px",
              display: "flex",
              alignItems: "center",
              padding: "0 3px",
              gap: "2px",
              background: "#1a1d22",
              borderBottom: "1px solid #111418",
            }}
          >
            <div
              style={{
                width: "3px",
                height: "3px",
                borderRadius: "999px",
                background: "#ff5f57",
              }}
            />
            <div
              style={{
                width: "3px",
                height: "3px",
                borderRadius: "999px",
                background: "#febc2e",
              }}
            />
            <div
              style={{
                width: "3px",
                height: "3px",
                borderRadius: "999px",
                background: "#28c840",
              }}
            />
          </div>

          <div
            style={{
              flex: 1,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              background:
                "radial-gradient(circle at 50% 35%, #101a24 0%, #0c0d0e 70%)",
            }}
          >
            <div
              style={{
                display: "flex",
                alignItems: "center",
              }}
            >
              <div
                style={{
                  background: "#70b8ff",
                  width: "3px",
                  height: "3px",
                  borderRadius: "1px",
                }}
              />
              <div
                style={{
                  marginLeft: "2px",
                  width: "7px",
                  height: "2px",
                  borderRadius: "999px",
                  background: "#70b8ff",
                }}
              />
              <div
                style={{
                  marginLeft: "2px",
                  width: "3px",
                  height: "9px",
                  borderRadius: "1px",
                  background: "#e8a838",
                }}
              />
            </div>
          </div>
        </div>
      </div>
    ),
    size
  );
}
