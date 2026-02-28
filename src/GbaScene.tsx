import { useEffect, useRef } from "react";
import * as THREE from "three";

type Bobber = {
  mesh: THREE.Mesh;
  baseY: number;
  speed: number;
  amp: number;
};

export default function GbaScene() {
  const mountRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) {
      return;
    }

    const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.5));
    renderer.setSize(mount.clientWidth, mount.clientHeight);
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    mount.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.fog = new THREE.FogExp2(0x0a051d, 0.04);

    const camera = new THREE.PerspectiveCamera(55, mount.clientWidth / mount.clientHeight, 0.1, 200);
    camera.position.set(0, 3, 15);
    camera.lookAt(0, 0, 0);

    const ambient = new THREE.AmbientLight(0xffffff, 0.4);
    scene.add(ambient);

    const keyLight = new THREE.DirectionalLight(0xff0077, 2.5); // Neon Pink
    keyLight.position.set(-6, 5, 5);
    scene.add(keyLight);

    const rimLight = new THREE.DirectionalLight(0x00f3ff, 2); // Neon Cyan
    rimLight.position.set(6, 4, -4);
    scene.add(rimLight);

    // Synthwave flowing grid floor
    const planeGeometry = new THREE.PlaneGeometry(100, 100, 50, 50);
    const planeMaterial = new THREE.MeshBasicMaterial({ 
      color: 0xdb00ff, 
      wireframe: true, 
      transparent: true, 
      opacity: 0.3 
    });
    const floor = new THREE.Mesh(planeGeometry, planeMaterial);
    floor.rotation.x = -Math.PI / 2;
    floor.position.y = -2;
    scene.add(floor);

    // Grid animation setup
    const initialFloorVertices: { x: number, y: number, z: number }[] = [];
    const positions = floor.geometry.attributes.position;
    for (let i = 0; i < positions.count; i++) {
        initialFloorVertices.push({
            x: positions.getX(i),
            y: positions.getY(i),
            z: positions.getZ(i),
        });
    }

    const spriteGroup = new THREE.Group();
    scene.add(spriteGroup);

    // Floating neon geometry
    const palette = [0x00f3ff, 0xff0077, 0xdb00ff, 0xffcc00];
    const bobbers: Bobber[] = [];
    
    for (let i = 0; i < 20; i += 1) {
      const shape = i % 4;
      let geometry: THREE.BufferGeometry;
      
      if (shape === 0) {
          geometry = new THREE.IcosahedronGeometry(0.5 + Math.random() * 0.5, 0);
      } else if (shape === 1) {
          geometry = new THREE.ConeGeometry(0.5, 1.2, 4);
      } else if (shape === 2) {
          geometry = new THREE.BoxGeometry(0.8, 0.8, 0.8);
      } else {
          geometry = new THREE.TorusGeometry(0.5, 0.15, 8, 12);
      }

      // Polished dark glossy material with neon emissive glow
      const material = new THREE.MeshStandardMaterial({
        color: 0x111111,
        emissive: palette[i % palette.length],
        emissiveIntensity: 0.6,
        metalness: 0.8,
        roughness: 0.2,
      });

      // Wireframe overlay for the retro arcade feel
      const mesh = new THREE.Mesh(geometry, material);
      const wireframeMaterial = new THREE.MeshBasicMaterial({ color: palette[i % palette.length], wireframe: true, transparent: true, opacity: 0.8 });
      const wireframe = new THREE.Mesh(geometry, wireframeMaterial);
      mesh.add(wireframe);

      const angle = (i / 20) * Math.PI * 2;
      const radius = 6 + Math.random() * 8;
      const y = 0 + Math.random() * 4;
      
      mesh.position.set(Math.cos(angle) * radius, y, Math.sin(angle) * radius - 2);
      mesh.rotation.set(Math.random(), Math.random() * Math.PI, Math.random());
      
      spriteGroup.add(mesh);
      bobbers.push({
        mesh,
        baseY: y,
        speed: 0.5 + Math.random() * 1.5,
        amp: 0.2 + Math.random() * 0.5,
      });
    }

    // Distant star dots
    const starGeom = new THREE.BufferGeometry();
    const starCount = 400;
    const starPos = new Float32Array(starCount * 3);
    for (let i = 0; i < starCount; i += 1) {
      const i3 = i * 3;
      starPos[i3] = (Math.random() - 0.5) * 80;
      starPos[i3 + 1] = Math.random() * 30 + 5;
      starPos[i3 + 2] = (Math.random() - 0.5) * 80 - 10;
    }
    starGeom.setAttribute("position", new THREE.BufferAttribute(starPos, 3));
    const stars = new THREE.Points(
      starGeom,
      new THREE.PointsMaterial({ color: 0x00f3ff, size: 0.08, transparent: true, opacity: 0.6 })
    );
    scene.add(stars);

    const clock = new THREE.Clock();
    let raf = 0;
    
    // Animate
    const animate = () => {
      const t = clock.getElapsedTime();
      
      spriteGroup.rotation.y = t * 0.1;
      
      for (const bob of bobbers) {
        bob.mesh.position.y = bob.baseY + Math.sin(t * bob.speed) * bob.amp;
        bob.mesh.rotation.x += 0.005;
        bob.mesh.rotation.y += 0.008;
      }
      
      // Animate flowing grid displacement
      for (let i = 0; i < positions.count; i++) {
          const ix = initialFloorVertices[i].x;
          // Creates a flowing terrain synthwave effect
          const zShift = Math.sin((ix + t * 4) * 0.2) * 0.8 + Math.cos((initialFloorVertices[i].y + t * 2) * 0.15) * 1.2;
          positions.setZ(i, zShift);
      }
      positions.needsUpdate = true;
      floor.position.z = (t * 8) % 2; // Move grid towards camera smoothly to create infinite scroll effect

      stars.rotation.y = t * 0.005;
      
      renderer.render(scene, camera);
      raf = window.requestAnimationFrame(animate);
    };
    animate();

    const handleResize = () => {
      if (!mount) {
        return;
      }
      const w = mount.clientWidth;
      const h = mount.clientHeight;
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      renderer.setSize(w, h);
    };
    window.addEventListener("resize", handleResize);

    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener("resize", handleResize);
      starGeom.dispose();
      for (const bob of bobbers) {
        bob.mesh.geometry.dispose();
        // Since mesh has multiple materials due to wireframe child, we clean up
        (bob.mesh.material as THREE.Material).dispose();
        bob.mesh.children.forEach(child => {
            if (child instanceof THREE.Mesh) {
                child.geometry.dispose();
                (child.material as THREE.Material).dispose();
            }
        });
      }
      floor.geometry.dispose();
      (floor.material as THREE.Material).dispose();
      renderer.dispose();
      if (renderer.domElement.parentElement === mount) {
        mount.removeChild(renderer.domElement);
      }
    };
  }, []);

  return <div className="gba-scene" ref={mountRef} aria-hidden="true" />;
}
